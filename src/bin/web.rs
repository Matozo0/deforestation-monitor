//! Interface local: upload TIFF em streaming, processamento Rust e painel integrado.
use std::{collections::HashMap,fs::{self,File},io::{Read,Write},net::{TcpListener,TcpStream},path::{Path,PathBuf},process::Command,sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},thread,time::Duration};
const HOME:&str=include_str!("web_home.html");
const OUT:&str="resultado_temporal";
const UPLOAD:&str="web_uploads";
const MAX_FILE:u64=5*1024*1024*1024;
fn response(s:&mut TcpStream,code:&str,mime:&str,data:&[u8]){
    let hdr=format!("HTTP/1.1 {code}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",data.len());
    let _=s.write_all(hdr.as_bytes());let _=s.write_all(data);
}
fn url_decode(s:&str)->String{
    let b=s.as_bytes();let mut out=Vec::new();let mut i=0;
    while i<b.len(){if b[i]==b'%'&&i+2<b.len(){let nib=|x:u8|match x{b'0'..=b'9'=>Some(x-b'0'),b'a'..=b'f'=>Some(x-b'a'+10),b'A'..=b'F'=>Some(x-b'A'+10),_=>None};if let(Some(a),Some(c))=(nib(b[i+1]),nib(b[i+2])){out.push(a*16+c);i+=3;continue;}}out.push(if b[i]==b'+'{b' '}else{b[i]});i+=1;}
    String::from_utf8_lossy(&out).to_string()
}
fn fields(s:&str)->HashMap<String,String>{s.split('&').filter_map(|v|v.split_once('=')).map(|(k,v)|(url_decode(k),url_decode(v))).collect()}
fn allowed_slot(s:&str)->bool{matches!(s,"t1_b04"|"t1_b08"|"t1_scl"|"t2_b04"|"t2_b08"|"t2_scl")}
fn header_value<'a>(h:&'a str,key:&str)->Option<&'a str>{h.lines().filter_map(|line|line.split_once(':')).find(|(k,_)|k.eq_ignore_ascii_case(key)).map(|(_,v)|v.trim())}
fn base()->PathBuf{PathBuf::from(env!("CARGO_MANIFEST_DIR"))}
fn has_tiff_signature(path:&Path)->bool{
    let mut signature=[0u8;4];
    File::open(path).and_then(|mut file|file.read_exact(&mut signature)).is_ok()
        && matches!(signature,[b'I',b'I',42,0]|[b'M',b'M',0,42]|[b'I',b'I',43,0]|[b'M',b'M',0,43])
}
fn valid_process_numbers(values:&[String])->bool{
    let parsed=values.iter().map(|value|value.parse::<i32>()).collect::<Result<Vec<_>,_>>();
    let Ok(parsed)=parsed else{return false};
    parsed.len()==7
        && parsed[0]>=0 && parsed[1]>=0
        && parsed[0]%2==0 && parsed[1]%2==0
        && (-1000..=1000).contains(&parsed[2])
        && parsed[3..].iter().all(|value|(-1_000_000..=1_000_000).contains(value))
}
fn handle(mut s:TcpStream,busy:Arc<AtomicBool>,status:Arc<Mutex<String>>){
    let _=s.set_read_timeout(Some(Duration::from_secs(120)));
    let mut pre=Vec::new();let mut part=[0u8;65536];let end=loop{
        if pre.len()>32768{return;}let n=match s.read(&mut part){Ok(n)=>n,Err(_)=>return};if n==0{return;}
        pre.extend_from_slice(&part[..n]);if let Some(i)=pre.windows(4).position(|w|w==b"\r\n\r\n"){break i+4;}
    };
    let h=match std::str::from_utf8(&pre[..end]){Ok(x)=>x,Err(_)=>return};
    let first=h.lines().next().unwrap_or("");let mut parts=first.split_whitespace();
    let method=parts.next().unwrap_or("");let full_route=parts.next().unwrap_or("/");
    let (route,query)=full_route.split_once('?').unwrap_or((full_route,""));
    let base=base();
    if method=="GET"{
        match route{
            "/"=>{response(&mut s,"200 OK","text/html; charset=utf-8",HOME.as_bytes());return;}
            "/status"=>{let x=status.lock().unwrap().clone();response(&mut s,"200 OK","text/plain; charset=utf-8",x.as_bytes());return;}
            _=>{}
        }
        if let Some(name)=route.strip_prefix("/files/"){
            let safe=!name.is_empty()&&name.chars().all(|x|x.is_ascii_alphanumeric()||"._-".contains(x))&&!name.contains("..");
            if !safe {response(&mut s,"403 Forbidden","text/plain",b"Caminho invalido");return;}
            let p=base.join(OUT).join(name);
            if let Ok(bytes)=fs::read(p){let mime=if name.ends_with(".png"){"image/png"}else if name.ends_with(".csv"){"text/csv; charset=utf-8"}else if name.ends_with(".html"){"text/html; charset=utf-8"}else{"application/octet-stream"};response(&mut s,"200 OK",mime,&bytes);return;}
        }
        response(&mut s,"404 Not Found","text/plain; charset=utf-8",b"Arquivo nao disponivel");return;
    }
    if method!="POST"{response(&mut s,"405 Method Not Allowed","text/plain",b"Metodo invalido");return;}
    if busy.load(Ordering::SeqCst){response(&mut s,"409 Conflict","text/plain",b"Processamento em andamento");return;}
    let size=match header_value(h,"Content-Length").and_then(|x|x.parse::<u64>().ok()){Some(v)=>v,None=>{response(&mut s,"411 Length Required","text/plain",b"Content-Length obrigatorio");return;}};
    if route=="/copy"{
        let p=fields(query);let from=p.get("from").cloned().unwrap_or_default();let to=p.get("to").cloned().unwrap_or_default();
        if !allowed_slot(&from)||!allowed_slot(&to)||from==to {response(&mut s,"400 Bad Request","text/plain",b"Banda invalida");return;}
        let a=base.join(UPLOAD).join(format!("{from}.tif"));let b=base.join(UPLOAD).join(format!("{to}.tif"));
        match fs::copy(a,b){Ok(_)=>response(&mut s,"200 OK","text/plain",b"Copiado"),Err(_)=>response(&mut s,"400 Bad Request","text/plain",b"Arquivo de origem ausente")};return;
    }
    if route=="/upload"{
        let slot=fields(query).get("slot").cloned().unwrap_or_default();
        if !allowed_slot(&slot){response(&mut s,"400 Bad Request","text/plain",b"Banda invalida");return;}
        if size==0||size>MAX_FILE{response(&mut s,"413 Payload Too Large","text/plain",b"Tamanho invalido");return;}
        let folder=base.join(UPLOAD);if fs::create_dir_all(&folder).is_err(){response(&mut s,"500 Internal Server Error","text/plain",b"Pasta nao pode ser criada");return;}
        let part_path=folder.join(format!("{slot}.part"));let dest=folder.join(format!("{slot}.tif"));
        let mut f=match File::create(&part_path){Ok(f)=>f,Err(_)=>{response(&mut s,"500 Internal Server Error","text/plain",b"Erro criando arquivo");return;}};
        let mut remaining=size;let from_header=&pre[end..];let n=(from_header.len() as u64).min(remaining) as usize;
        if f.write_all(&from_header[..n]).is_err(){return;}remaining-=n as u64;
        while remaining>0{
            let want=(remaining as usize).min(part.len());let count=match s.read(&mut part[..want]){Ok(x)=>x,Err(_)=>0};
            if count==0{break;}if f.write_all(&part[..count]).is_err(){break;}remaining-=count as u64;
        }
        drop(f);
        if remaining>0{let _=fs::remove_file(part_path);response(&mut s,"400 Bad Request","text/plain",b"Upload interrompido");return;}
        if !has_tiff_signature(&part_path){let _=fs::remove_file(part_path);response(&mut s,"400 Bad Request","text/plain; charset=utf-8",b"Arquivo enviado nao possui assinatura TIFF valida");return;}
        if fs::rename(&part_path,&dest).is_err(){let _=fs::remove_file(&dest);if fs::rename(&part_path,&dest).is_err(){response(&mut s,"500 Internal Server Error","text/plain",b"Erro salvando TIFF");return;}}
        response(&mut s,"200 OK","text/plain; charset=utf-8",format!("{slot}: recebido ({:.1} MB)",size as f64/1e6).as_bytes());return;
    }
    if route!="/process"{response(&mut s,"404 Not Found","text/plain",b"Rota inexistente");return;}
    if size==0||size>16384{response(&mut s,"400 Bad Request","text/plain",b"Formulario invalido");return;}
    while (pre.len()-end) < size as usize{let n=match s.read(&mut part){Ok(v)=>v,Err(_)=>0};if n==0{break;}pre.extend_from_slice(&part[..n]);}
    if pre.len()-end<size as usize{response(&mut s,"400 Bad Request","text/plain",b"Formulario incompleto");return;}
    let form=fields(&String::from_utf8_lossy(&pre[end..end+size as usize]));
    let keys=["row","col","threshold","o1r","o1n","o2r","o2n"];
    let nums:Vec<String>=keys.iter().map(|k|form.get(*k).cloned().unwrap_or_default()).collect();
    if !valid_process_numbers(&nums){
        response(&mut s,"400 Bad Request","text/plain",b"Parametros numericos invalidos");return;
    }
    let slots=["t1_b04","t1_b08","t1_scl","t2_b04","t2_b08","t2_scl"];
    for slot in slots{if !base.join(UPLOAD).join(format!("{slot}.tif")).is_file(){response(&mut s,"400 Bad Request","text/plain; charset=utf-8",format!("Falta selecionar {slot}").as_bytes());return;}}
    if busy.swap(true,Ordering::SeqCst){response(&mut s,"409 Conflict","text/plain",b"Ja processando");return;}
    *status.lock().unwrap()="PROCESSANDO: calculando NDVI, mascaras, mudancas e areas...".into();
    let st=status.clone();let flag=busy.clone();
    thread::spawn(move||{
        let mut args:Vec<String>=slots.iter().map(|slot|format!("{UPLOAD}/{slot}.tif")).collect();args.extend(nums);
        let outcome=Command::new("cargo").current_dir(&base).arg("run").arg("--quiet").arg("--bin").arg("temporal").arg("--").args(args).output();
        let msg=match outcome{Ok(x) if x.status.success()=>"CONCLUIDO: resultados prontos".to_owned(),Ok(x)=>format!("ERRO: {}",String::from_utf8_lossy(&x.stderr).chars().take(3000).collect::<String>()),Err(e)=>format!("ERRO: {e}")};
        *st.lock().unwrap()=msg;flag.store(false,Ordering::SeqCst);
    });
    response(&mut s,"202 Accepted","text/plain",b"Processamento iniciado");
}
fn main(){
    let listener=TcpListener::bind("127.0.0.1:3000").expect("Porta 3000 em uso");
    println!("Painel: http://127.0.0.1:3000");println!("Use Ctrl+C para encerrar");
    let busy=Arc::new(AtomicBool::new(false));let status=Arc::new(Mutex::new("Pronto para receber imagens".into()));
    for conn in listener.incoming(){if let Ok(s)=conn{let b=busy.clone();let st=status.clone();thread::spawn(move||handle(s,b,st));}}
}

#[cfg(test)]
mod tests{
    use super::*;

    #[test]
    fn accepts_valid_process_numbers(){
        let values=["2200","7200","400","0","-1000","0","-1000"].map(str::to_owned);
        assert!(valid_process_numbers(&values));
    }

    #[test]
    fn rejects_invalid_process_numbers(){
        for values in [
            ["-2","7200","400","0","0","0","0"],
            ["2201","7200","400","0","0","0","0"],
            ["2200","7200","1001","0","0","0","0"],
            ["2200","7200","400","x","0","0","0"],
        ]{
            assert!(!valid_process_numbers(&values.map(str::to_owned)));
        }
    }

    #[test]
    fn recognizes_tiff_signatures(){
        for signature in [[b'I',b'I',42,0],[b'M',b'M',0,42],[b'I',b'I',43,0],[b'M',b'M',0,43]]{
            assert!(matches!(signature,[b'I',b'I',42,0]|[b'M',b'M',0,42]|[b'I',b'I',43,0]|[b'M',b'M',0,43]));
        }
        assert!(!matches!(*b"nope",[b'I',b'I',42,0]|[b'M',b'M',0,42]|[b'I',b'I',43,0]|[b'M',b'M',0,43]));
    }
}
