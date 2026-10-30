//! Interface local para selecionar cenas existentes em data/ e executar o pipeline Rust.
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const HOME: &str = include_str!("web_home.html");
const OUT: &str = "resultado_temporal";
const REQUIRED: [&str; 3] = ["B04.tif", "B08.tif", "SCL.tif"];

fn response(stream: &mut TcpStream, code: &str, mime: &str, data: &[u8]) {
    let header = format!(
        "HTTP/1.1 {code}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        data.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(data);
}

fn url_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            if let (Some(a), Some(b)) = (nibble(bytes[index + 1]), nibble(bytes[index + 2])) {
                output.push(a * 16 + b);
                index += 3;
                continue;
            }
        }
        output.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8_lossy(&output).to_string()
}

fn fields(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter_map(|item| item.split_once('='))
        .map(|(key, value)| (url_decode(key), url_decode(value)))
        .collect()
}
fn header_value<'a>(headers: &'a str, key: &str) -> Option<&'a str> {
    headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.trim())
}
fn base() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn has_tiff_signature(path: &Path) -> bool {
    let mut signature = [0u8; 4];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut signature))
        .is_ok()
        && matches!(
            signature,
            [b'I', b'I', 42, 0] | [b'M', b'M', 0, 42] | [b'I', b'I', 43, 0] | [b'M', b'M', 0, 43]
        )
}
fn scene_date(name: &str) -> Option<String> {
    name.split('_')
        .find(|part| part.len() == 8 && part.bytes().all(|byte| byte.is_ascii_digit()))
        .map(|date| format!("{}/{}/{}", &date[6..8], &date[4..6], &date[0..4]))
}
fn safe_scene_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}
fn scene_files(root: &Path, name: &str) -> Result<[PathBuf; 3], String> {
    if !safe_scene_name(name) {
        return Err("Nome de cena invalido".into());
    }
    let data = root
        .join("data")
        .canonicalize()
        .map_err(|_| "Pasta data/ nao encontrada")?;
    let scene = data
        .join(name)
        .canonicalize()
        .map_err(|_| format!("Cena nao encontrada: {name}"))?;
    if !scene.starts_with(&data) || !scene.is_dir() {
        return Err("Cena fora da pasta data/".into());
    }
    let files = REQUIRED.map(|file| scene.join(file));
    for (index, path) in files.iter().enumerate() {
        if !path.is_file() {
            return Err(format!("Cena {name}: arquivo {} ausente", REQUIRED[index]));
        }
        if !has_tiff_signature(path) {
            return Err(format!(
                "Cena {name}: {} nao e um TIFF legivel",
                REQUIRED[index]
            ));
        }
    }
    Ok(files)
}
fn list_scenes(root: &Path) -> Result<serde_json::Value, String> {
    let entries = fs::read_dir(root.join("data"))
        .map_err(|error| format!("Nao foi possivel ler data/: {error}"))?;
    let mut scenes = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !safe_scene_name(&name) {
            continue;
        }
        let missing: Vec<&str> = REQUIRED
            .iter()
            .copied()
            .filter(|file| !entry.path().join(file).is_file())
            .collect();
        scenes.push(serde_json::json!({"name":name,"date":scene_date(&name),"complete":missing.is_empty(),"missing":missing,"files":REQUIRED}));
    }
    scenes.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(serde_json::Value::Array(scenes))
}
fn valid_process_numbers(values: &[String]) -> bool {
    let Ok(parsed) = values
        .iter()
        .map(|value| value.parse::<i32>())
        .collect::<Result<Vec<_>, _>>()
    else {
        return false;
    };
    parsed.len() == 7
        && parsed[0] >= 0
        && parsed[1] >= 0
        && parsed[0] % 2 == 0
        && parsed[1] % 2 == 0
        && (-1000..=1000).contains(&parsed[2])
        && parsed[3..]
            .iter()
            .all(|value| (-1_000_000..=1_000_000).contains(value))
}
fn temporal_command(
    root: &Path,
    validate_only: bool,
    files: [&Path; 6],
    numbers: &[String],
) -> std::io::Result<std::process::Output> {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .arg("run")
        .arg("--quiet")
        .arg("--bin")
        .arg("temporal")
        .arg("--");
    if validate_only {
        command.arg("--validate-only");
    }
    command.args(files).args(numbers).output()
}
fn command_error(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let message = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    message.trim().chars().take(3000).collect()
}
fn read_body(
    stream: &mut TcpStream,
    request: &mut Vec<u8>,
    header_end: usize,
    size: usize,
) -> Result<String, ()> {
    let mut buffer = [0u8; 8192];
    while request.len() - header_end < size {
        let count = stream.read(&mut buffer).map_err(|_| ())?;
        if count == 0 {
            return Err(());
        }
        request.extend_from_slice(&buffer[..count]);
    }
    Ok(String::from_utf8_lossy(&request[header_end..header_end + size]).to_string())
}

fn handle(mut stream: TcpStream, busy: Arc<AtomicBool>, status: Arc<Mutex<String>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let mut request = Vec::new();
    let mut buffer = [0u8; 8192];
    let header_end = loop {
        if request.len() > 32768 {
            return;
        }
        let Ok(count) = stream.read(&mut buffer) else {
            return;
        };
        if count == 0 {
            return;
        }
        request.extend_from_slice(&buffer[..count]);
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let Ok(headers) = std::str::from_utf8(&request[..header_end]) else {
        return;
    };
    let mut first = headers.lines().next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("").to_owned();
    let route = first
        .next()
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/")
        .to_owned();
    let content_length =
        header_value(headers, "Content-Length").and_then(|value| value.parse::<usize>().ok());
    let root = base();
    if method == "GET" {
        if route == "/" {
            response(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                HOME.as_bytes(),
            );
            return;
        }
        if route == "/status" {
            let message = status.lock().unwrap().clone();
            response(
                &mut stream,
                "200 OK",
                "text/plain; charset=utf-8",
                message.as_bytes(),
            );
            return;
        }
        if route == "/scenes" {
            match list_scenes(&root) {
                Ok(value) => response(
                    &mut stream,
                    "200 OK",
                    "application/json; charset=utf-8",
                    value.to_string().as_bytes(),
                ),
                Err(error) => response(
                    &mut stream,
                    "500 Internal Server Error",
                    "text/plain; charset=utf-8",
                    error.as_bytes(),
                ),
            }
            return;
        }
        if let Some(name) = route.strip_prefix("/files/") {
            let safe = !name.is_empty()
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || "._-".contains(ch))
                && !name.contains("..");
            if !safe {
                response(
                    &mut stream,
                    "403 Forbidden",
                    "text/plain",
                    b"Caminho invalido",
                );
                return;
            }
            if let Ok(bytes) = fs::read(root.join(OUT).join(name)) {
                let mime = if name.ends_with(".png") {
                    "image/png"
                } else if name.ends_with(".csv") {
                    "text/csv; charset=utf-8"
                } else {
                    "text/html; charset=utf-8"
                };
                response(&mut stream, "200 OK", mime, &bytes);
                return;
            }
        }
        response(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"Arquivo nao disponivel",
        );
        return;
    }
    if method != "POST" {
        response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"Metodo invalido",
        );
        return;
    }
    if busy.load(Ordering::SeqCst) {
        response(
            &mut stream,
            "409 Conflict",
            "text/plain",
            b"Processamento em andamento",
        );
        return;
    }
    let Some(size) = content_length else {
        response(
            &mut stream,
            "411 Length Required",
            "text/plain",
            b"Content-Length obrigatorio",
        );
        return;
    };
    if size == 0 || size > 16384 {
        response(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"Formulario invalido",
        );
        return;
    }
    let Ok(body) = read_body(&mut stream, &mut request, header_end, size) else {
        response(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"Formulario incompleto",
        );
        return;
    };
    let form = fields(&body);
    let t1 = form.get("t1").cloned().unwrap_or_default();
    let t2 = form.get("t2").cloned().unwrap_or_default();
    let keys = ["row", "col", "threshold", "o1r", "o1n", "o2r", "o2n"];
    let numbers: Vec<String> = keys
        .iter()
        .map(|key| form.get(*key).cloned().unwrap_or_default())
        .collect();
    if !valid_process_numbers(&numbers) {
        response(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"Parametros numericos invalidos",
        );
        return;
    }
    let files1 = match scene_files(&root, &t1) {
        Ok(files) => files,
        Err(error) => {
            response(
                &mut stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                error.as_bytes(),
            );
            return;
        }
    };
    let files2 = match scene_files(&root, &t2) {
        Ok(files) => files,
        Err(error) => {
            response(
                &mut stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                error.as_bytes(),
            );
            return;
        }
    };
    let files = [
        &*files1[0],
        &*files1[1],
        &*files1[2],
        &*files2[0],
        &*files2[1],
        &*files2[2],
    ];
    if route == "/validate" {
        match temporal_command(&root, true, files, &numbers) {
            Ok(output) if output.status.success() => response(
                &mut stream,
                "200 OK",
                "text/plain; charset=utf-8",
                output.stdout.as_slice(),
            ),
            Ok(output) => response(
                &mut stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                command_error(&output).as_bytes(),
            ),
            Err(error) => response(
                &mut stream,
                "500 Internal Server Error",
                "text/plain; charset=utf-8",
                error.to_string().as_bytes(),
            ),
        }
        return;
    }
    if route != "/process" {
        response(
            &mut stream,
            "404 Not Found",
            "text/plain",
            b"Rota inexistente",
        );
        return;
    }
    if busy.swap(true, Ordering::SeqCst) {
        response(&mut stream, "409 Conflict", "text/plain", b"Ja processando");
        return;
    }
    let _ = fs::remove_dir_all(root.join(OUT));
    *status.lock().unwrap() = format!("PROCESSANDO: {t1} -> {t2}");
    let state = status.clone();
    let flag = busy.clone();
    let owned = files.map(Path::to_path_buf);
    thread::spawn(move || {
        let refs = [
            &*owned[0], &*owned[1], &*owned[2], &*owned[3], &*owned[4], &*owned[5],
        ];
        let message = match temporal_command(&root, false, refs, &numbers) {
            Ok(output) if output.status.success() => "CONCLUIDO: resultados prontos".to_owned(),
            Ok(output) => format!("ERRO: {}", command_error(&output)),
            Err(error) => format!("ERRO: {error}"),
        };
        *state.lock().unwrap() = message;
        flag.store(false, Ordering::SeqCst);
    });
    response(
        &mut stream,
        "202 Accepted",
        "text/plain",
        b"Processamento iniciado",
    );
}
fn main() {
    let listener = TcpListener::bind("127.0.0.1:3000").expect("Porta 3000 em uso");
    println!("Painel: http://127.0.0.1:3000");
    println!("Use Ctrl+C para encerrar");
    let busy = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new("Pronto para selecionar cenas em data/".into()));
    for stream in listener.incoming().flatten() {
        let flag = busy.clone();
        let state = status.clone();
        thread::spawn(move || handle(stream, flag, state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_date() {
        assert_eq!(
            scene_date("S2B_21MXM_20260912_0_L2A").as_deref(),
            Some("12/09/2026")
        );
    }
    #[test]
    fn validates_numbers() {
        assert!(valid_process_numbers(
            &["2200", "7200", "400", "0", "-1000", "0", "-1000"].map(str::to_owned)
        ));
        assert!(!valid_process_numbers(
            &["2201", "7200", "400", "0", "0", "0", "0"].map(str::to_owned)
        ));
    }
    #[test]
    fn rejects_missing_file() {
        let root =
            std::env::temp_dir().join(format!("deforestation-monitor-web-{}", std::process::id()));
        let scene = root.join("data").join("S2B_21MXM_20260912_0_L2A");
        fs::create_dir_all(&scene).unwrap();
        fs::write(scene.join("B04.tif"), [b'I', b'I', 42, 0]).unwrap();
        fs::write(scene.join("B08.tif"), [b'I', b'I', 42, 0]).unwrap();
        let error = scene_files(&root, "S2B_21MXM_20260912_0_L2A").unwrap_err();
        assert!(error.contains("SCL.tif"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_paths() {
        assert!(!safe_scene_name("../outside"));
        assert!(!safe_scene_name("folder/scene"));
        assert!(safe_scene_name("S2A_21MXM_20250805_0_L2A"));
    }
}
