use geotiff_reader::GeoTiffFile;
use image::{GrayImage, Luma, Rgb, RgbImage};
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::fs;
use tiff_reader::TiffFile;

struct SceneMasks {
    valid: Vec<u8>,
    vegetation: Vec<u8>,
    ndvi_scaled: Vec<i16>,
    red_preview: Vec<u8>,
    nir_preview: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Point {
    x: usize,
    y: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Edge {
    start: Point,
    end: Point,
}

fn generate_grayimage(mask: &[u8], size: u32, path: &str) {
    let pixels: Vec<u8> = mask.iter().map(|&v| if v == 1 { 255 } else { 0 }).collect();
    let img = GrayImage::from_raw(size as u32, size as u32, pixels).unwrap();
    img.save(path).expect("Falha ao salvar a mascara");
}

fn generate_edges_image(edges: &[Edge], size: usize, path: &str) {
    let width = (size + 1) as u32;
    let height = (size + 1) as u32;

    let mut img = GrayImage::new(width, height);

    for edge in edges {
        let x1 = edge.start.x;
        let y1 = edge.start.y;
        let x2 = edge.end.x;
        let y2 = edge.end.y;

        // Aresta vertical
        if x1 == x2 {
            let x = x1 as u32;
            let y_start = y1.min(y2);
            let y_end = y1.max(y2);

            for y in y_start..=y_end {
                img.put_pixel(x, y as u32, Luma([255]));
            }
        }
        // Aresta horizontal
        else if y1 == y2 {
            let y = y1 as u32;
            let x_start = x1.min(x2);
            let x_end = x1.max(x2);

            for x in x_start..=x_end {
                img.put_pixel(x as u32, y, Luma([255]));
            }
        }
    }

    img.save(path).unwrap();
}

fn process_initial_scene(
    b04_path: &str,
    b08_path: &str,
    scl_path: &str,
    row_offset: usize,
    col_offset: usize,
    size: usize,
    threshold: i32,
    debug: bool,
    offset_red: i32,
    offset_nir: i32,
) -> SceneMasks {
    let scale: usize = 2;
    let scl_row_offset = row_offset / scale;
    let scl_col_offset = col_offset / scale;
    let scl_size = size / scale;

    let b04 = TiffFile::open(b04_path).unwrap();
    let b08 = TiffFile::open(b08_path).unwrap();
    let scl = TiffFile::open(scl_path).unwrap();
    let tile_b04: ndarray::ArrayD<u16> = b04
        .read_window(0, row_offset, col_offset, size, size)
        .unwrap();
    let tile_b08: ndarray::ArrayD<u16> = b08
        .read_window(0, row_offset, col_offset, size, size)
        .unwrap();
    let tile_scl: ndarray::ArrayD<u8> = scl
        .read_window(0, scl_row_offset, scl_col_offset, scl_size, scl_size)
        .unwrap();

    let mut valid_mask = vec![0u8; size * size];
    let mut vegetation_mask = vec![0u8; size * size];
    let mut ndvi_scaled = vec![i16::MIN; size * size];
    let mut red_preview = vec![0u8; size * size];
    let mut nir_preview = vec![0u8; size * size];

    let mut vegetation_count: usize = 0;
    let mut non_vegetation_count: usize = 0;
    let mut invalid_count: usize = 0;

    let mut min_ndvi = 1000;
    let mut max_ndvi = -1000;

    // Itera pelo B04, B08 e SCL para gerar a mascara valida e mascara de vegetação
    for i in 0..size {
        for j in 0..size {
            let index = i * size + j;

            let red_dn = tile_b04[[i, j]] as i32;
            let nir_dn = tile_b08[[i, j]] as i32;
            // Na especificacao Sentinel-2, DN=0 significa NoData.
            if red_dn == 0 || nir_dn == 0 {
                invalid_count += 1;
                continue;
            }
            let red = red_dn + offset_red;
            let nir = nir_dn + offset_nir;
            red_preview[index] = (red_dn / 40).clamp(0, 255) as u8;
            nir_preview[index] = (nir_dn / 40).clamp(0, 255) as u8;

            let scl_value = tile_scl[[i / scale, j / scale]];
            let valid = matches!(scl_value, 4 | 5); // Rejeita tudo menos os 4 (vegetação) e 5 (não vegetação)

            if !valid {
                invalid_count += 1;
                continue;
            }

            let denominator = nir + red;

            if denominator <= 0 {
                invalid_count += 1;
                continue;
            }

            valid_mask[index] = 1;

            let ndvi = ((nir - red) * 1000) / denominator;
            ndvi_scaled[index] = ndvi.clamp(i16::MIN as i32, i16::MAX as i32) as i16;

            if ndvi >= threshold {
                vegetation_count += 1;
                vegetation_mask[index] = 1;
            } else {
                non_vegetation_count += 1;
                vegetation_mask[index] = 0;
            }

            if ndvi < min_ndvi {
                min_ndvi = ndvi;
            }
            if ndvi > max_ndvi {
                max_ndvi = ndvi;
            }
        }
    }

    if debug {
        generate_grayimage(&vegetation_mask, size as u32, "./data/vegetation_mask.png");
        generate_grayimage(&valid_mask, size as u32, "./data/valid_mask.png");
        println!("Vegetação: {}", vegetation_count);
        println!("Não vegetação: {}", non_vegetation_count);
        println!("Inválidos: {}", invalid_count);
        println!("NDVI mínimo: {}", min_ndvi);
        println!("NDVI máximo: {}", max_ndvi);
    }

    SceneMasks {
        valid: valid_mask,
        vegetation: vegetation_mask,
        ndvi_scaled,
        red_preview,
        nir_preview,
    }
}

fn process_flood_fill(mask: &[u8], size: usize, _debug: bool) -> (Vec<u32>, u32) {
    let mut labels = vec![0u32; mask.len()];
    let mut current_label = 0u32;

    let directions = [
        (-1, 0), // cima
        (1, 0),  // baixo
        (0, -1), // esquerda
        (0, 1),  // direita
    ];

    for row in 0..size {
        for col in 0..size {
            let index = row * size + col;

            if mask[index] == 0 || labels[index] != 0 {
                continue;
            }

            current_label += 1;

            let mut queue = VecDeque::new();
            labels[index] = current_label;
            queue.push_back((row, col));

            while let Some((r, c)) = queue.pop_front() {
                for (dr, dc) in directions {
                    let nr = r as isize + dr;
                    let nc = c as isize + dc;

                    if nr < 0 || nc < 0 || nr >= size as isize || nc >= size as isize {
                        continue;
                    }

                    let nr = nr as usize;
                    let nc = nc as usize;
                    let nindex = nr * size + nc;

                    if mask[nindex] == 1 && labels[nindex] == 0 {
                        labels[nindex] = current_label;

                        queue.push_back((nr, nc));
                    }
                }
            }
        }
    }
    (labels, current_label)
}

fn process_edge(mask: &[u8], size: usize) -> Vec<Edge> {
    let mut edges: Vec<Edge> = Vec::new();

    for i in 0..size {
        for j in 0..size {
            let index = i * size + j;

            if mask[index] == 0 {
                continue;
            }

            // Aresta cima
            if i == 0 || mask[(i - 1) * size + j] == 0 {
                let edge = Edge {
                    start: Point { x: j, y: i },
                    end: Point { x: j + 1, y: i },
                };
                edges.push(edge);
            }
            // Aresta baixo
            if i + 1 == size || mask[(i + 1) * size + j] == 0 {
                let edge = Edge {
                    start: Point { x: j + 1, y: i + 1 },
                    end: Point { x: j, y: i + 1 },
                };
                edges.push(edge);
            }
            // Aresta esquerda
            if j == 0 || mask[i * size + (j - 1)] == 0 {
                let edge = Edge {
                    start: Point { x: j, y: i + 1 },
                    end: Point { x: j, y: i },
                };
                edges.push(edge);
            }
            // Aresta direita
            if j + 1 == size || mask[i * size + (j + 1)] == 0 {
                let edge = Edge {
                    start: Point { x: j + 1, y: i },
                    end: Point { x: j + 1, y: i + 1 },
                };
                edges.push(edge);
            }
        }
    }

    edges
}

// Direcoes no sistema de coordenadas da imagem (y cresce para baixo).
// 0 = direita, 1 = baixo, 2 = esquerda, 3 = cima.
fn edge_direction(edge: &Edge) -> usize {
    let a = edge.start;
    let b = edge.end;

    if b.x > a.x {
        0
    } else if b.y > a.y {
        1
    } else if b.x < a.x {
        2
    } else {
        3
    }
}

// Encontra todos os aneis (contornos externos e furos).
// A regra de preferir a curva a direita separa contornos que tocam um vertice.
fn connect_all_edges(edges: &[Edge]) -> Vec<Vec<Point>> {
    let mut contours = Vec::new();
    let mut visited = vec![false; edges.len()];
    let mut starts: HashMap<Point, Vec<usize>> = HashMap::new();

    for (index, edge) in edges.iter().enumerate() {
        starts.entry(edge.start).or_default().push(index);
    }

    for first in 0..edges.len() {
        if visited[first] {
            continue;
        }

        let start = edges[first].start;
        let mut current = first;
        let mut contour = Vec::new();

        loop {
            assert!(!visited[current], "Aresta repetida antes do fechamento");
            visited[current] = true;
            let edge = &edges[current];
            contour.push(edge.start);

            if edge.end == start {
                break;
            }

            let candidates = starts.get(&edge.end).expect("Contorno aberto");
            let direction = edge_direction(edge);

            let next = candidates
                .iter()
                .filter(|&&index| !visited[index])
                .min_by_key(|&&index| {
                    let next_direction = edge_direction(&edges[index]);
                    let turn = (next_direction + 4 - direction) % 4;
                    match turn {
                        1 => 0, // direita
                        0 => 1, // reto
                        3 => 2, // esquerda
                        _ => 3, // retorno
                    }
                });

            current = *next.expect("Contorno sem continuacao");
        }
        contours.push(contour);
    }
    contours
}

fn calculate_signed_area(polygon: &[Point]) -> f64 {
    if polygon.len() < 3 {
        return 0.0;
    }

    let mut sum: i128 = 0;
    for i in 0..polygon.len() {
        let j = (i + 1) % polygon.len();
        let a = polygon[i];
        let b = polygon[j];
        sum += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
    }
    sum as f64 / 2.0
}

#[cfg(test)]
fn calculate_polygon_area(polygon: &[Point]) -> f64 {
    calculate_signed_area(polygon).abs()
}

fn calculate_total_area(contours: &[Vec<Point>]) -> f64 {
    contours.iter().map(|c| calculate_signed_area(c)).sum()
}

// Resultado individual de uma componente identificada pelo Flood Fill.
#[derive(Debug)]
struct ComponentArea {
    label: u32,
    pixels: usize,
    contours: usize,
    area_pixels: f64,
}

// Versao didatica: reutiliza a extracao de arestas e o calculo de contornos
// ja testados, isolando cada componente em uma mascara temporaria.
#[cfg(test)]
fn calculate_component_areas(labels: &[u32], count: u32, size: usize) -> Vec<ComponentArea> {
    assert_eq!(
        labels.len(),
        size * size,
        "Dimensoes dos rotulos incorretas"
    );
    let mut results = Vec::new();

    for label in 1..=count {
        let component_mask: Vec<u8> = labels
            .iter()
            .map(|&value| if value == label { 1 } else { 0 })
            .collect();

        let pixels = component_mask.iter().filter(|&&v| v == 1).count();
        let edges = process_edge(&component_mask, size);
        let contours = connect_all_edges(&edges);
        let area_pixels = calculate_total_area(&contours);

        assert_eq!(
            area_pixels, pixels as f64,
            "Area por determinantes diferente da contagem na componente {}",
            label
        );

        results.push(ComponentArea {
            label,
            pixels,
            contours: contours.len(),
            area_pixels,
        });
    }
    results
}

// Transformacao afim lida do GeoTIFF.
// Os coeficientes sao lidos dos metadados uma vez e calculados manualmente.
// A mesma formula vale para rasters com rotacao (skew diferente de zero).
fn pixel_to_utm(
    point: Point,
    row_offset: usize,
    col_offset: usize,
    transform: &geotiff_reader::transform::GeoTransform,
) -> (f64, f64) {
    let col = (col_offset + point.x) as f64;
    let row = (row_offset + point.y) as f64;
    let east = transform.origin_x + transform.pixel_width * col + transform.skew_x * row;
    let north = transform.origin_y + transform.skew_y * col + transform.pixel_height * row;
    (east, north)
}

// Inversa aproximada da projecao UTM / Transversa de Mercator no WGS84.
// Esta implementacao usa a serie classica (ate D^6 para latitude e D^5 para longitude).
// Aceita somente WGS84 / UTM norte (EPSG:326xx) ou sul (EPSG:327xx).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UtmCrs {
    epsg: u32,
    zone: u8,
    northern: bool,
}

fn utm_crs_from_epsg(epsg: u32) -> Result<UtmCrs, String> {
    let (zone, northern) = match epsg {
        32601..=32660 => ((epsg - 32600) as u8, true),
        32701..=32760 => ((epsg - 32700) as u8, false),
        _ => {
            return Err(format!(
                "EPSG {epsg} nao suportado: esperado WGS84 / UTM 32601..32660 ou 32701..32760"
            ));
        }
    };
    Ok(UtmCrs {
        epsg,
        zone,
        northern,
    })
}

fn utm_to_latlon(easting: f64, northing: f64, crs: UtmCrs) -> (f64, f64) {
    let a: f64 = 6_378_137.0;
    let f: f64 = 1.0 / 298.257_223_563;
    let e2 = f * (2.0 - f);
    let ep2 = e2 / (1.0 - e2);
    let k0: f64 = 0.9996;
    let meridian = ((crs.zone as f64) * 6.0 - 183.0).to_radians();

    let x = easting - 500_000.0;
    let y = if crs.northern {
        northing
    } else {
        northing - 10_000_000.0
    };
    let m = y / k0;
    let mu = m / (a * (1.0 - e2 / 4.0 - 3.0 * e2.powi(2) / 64.0 - 5.0 * e2.powi(3) / 256.0));
    let e1 = (1.0 - (1.0 - e2).sqrt()) / (1.0 + (1.0 - e2).sqrt());

    let phi1 = mu
        + (3.0 * e1 / 2.0 - 27.0 * e1.powi(3) / 32.0) * (2.0 * mu).sin()
        + (21.0 * e1.powi(2) / 16.0 - 55.0 * e1.powi(4) / 32.0) * (4.0 * mu).sin()
        + 151.0 * e1.powi(3) / 96.0 * (6.0 * mu).sin()
        + 1097.0 * e1.powi(4) / 512.0 * (8.0 * mu).sin();

    let sin_phi = phi1.sin();
    let cos_phi = phi1.cos();
    let t1 = phi1.tan().powi(2);
    let c1 = ep2 * cos_phi.powi(2);
    let n1 = a / (1.0 - e2 * sin_phi.powi(2)).sqrt();
    let r1 = a * (1.0 - e2) / (1.0 - e2 * sin_phi.powi(2)).powf(1.5);
    let d = x / (n1 * k0);

    let lat = phi1
        - n1 * phi1.tan() / r1
            * (d.powi(2) / 2.0
                - (5.0 + 3.0 * t1 + 10.0 * c1 - 4.0 * c1.powi(2) - 9.0 * ep2) * d.powi(4) / 24.0
                + (61.0 + 90.0 * t1 + 298.0 * c1 + 45.0 * t1.powi(2)
                    - 252.0 * ep2
                    - 3.0 * c1.powi(2))
                    * d.powi(6)
                    / 720.0);
    let lon = meridian
        + (d - (1.0 + 2.0 * t1 + c1) * d.powi(3) / 6.0
            + (5.0 - 2.0 * c1 + 28.0 * t1 - 3.0 * c1.powi(2) + 8.0 * ep2 + 24.0 * t1.powi(2))
                * d.powi(5)
                / 120.0)
            / cos_phi;
    (lat.to_degrees(), lon.to_degrees())
}

fn show_geographic_coordinates(
    path: &str,
    size: usize,
    row_offset: usize,
    col_offset: usize,
    contours: &[Vec<Point>],
) {
    let geo = GeoTiffFile::open(path).expect("Falha ao abrir GeoTIFF para metadados");
    let epsg = geo.epsg().expect("GeoTIFF sem EPSG");
    let crs = utm_crs_from_epsg(epsg).expect("CRS UTM/WGS84 nao suportado");

    let transform = geo.transform().expect("GeoTIFF sem transformacao afim");
    let cell_area = (transform.pixel_width * transform.pixel_height
        - transform.skew_x * transform.skew_y)
        .abs();
    println!("\n--- COORDENADAS DO RECORTE ---");
    println!(
        "EPSG: {}; fuso UTM {}{}; area projetada de cada pixel: {cell_area:.4} m²",
        crs.epsg,
        crs.zone,
        if crs.northern { "N" } else { "S" }
    );
    for (name, point) in [
        ("Canto superior esquerdo", Point { x: 0, y: 0 }),
        ("Canto superior direito", Point { x: size, y: 0 }),
        ("Canto inferior esquerdo", Point { x: 0, y: size }),
        ("Canto inferior direito", Point { x: size, y: size }),
        (
            "Centro aproximado",
            Point {
                x: size / 2,
                y: size / 2,
            },
        ),
    ] {
        let (east, north) = pixel_to_utm(point, row_offset, col_offset, &transform);
        let (lat, lon) = utm_to_latlon(east, north, crs);
        println!("{name}: UTM ({east:.2}, {north:.2}) m; lat {lat:.7}, lon {lon:.7}");
    }

    // Exemplo concreto: o primeiro vertice do primeiro contorno.
    if let Some(first) = contours.first().and_then(|c| c.first()) {
        let (east, north) = pixel_to_utm(*first, row_offset, col_offset, &transform);
        let (lat, lon) = utm_to_latlon(east, north, crs);
        println!(
            "Primeiro vertice do primeiro contorno: pixel ({}, {}), UTM ({east:.2}, {north:.2}) m, lat {lat:.7}, lon {lon:.7}",
            first.x, first.y
        );
    }
}

const SIZE: usize = 512;
const ROW_OFFSET: usize = 2200;
const COL_OFFSET: usize = 7200;
const NDVI_THRESHOLD: i32 = 400; // 0,4

// IMPORTANTISSIMO: confirmar no MTD_MSIL2A.xml de cada cena.
// Se o TIFF ja foi harmonizado para refletancia sem offset, manter zero.
// Produtos L2A recentes nao harmonizados podem exigir -1000.
const T1_OFFSET_B04: i32 = 0;
const T1_OFFSET_B08: i32 = 0;
const T2_OFFSET_B04: i32 = 0;
const T2_OFFSET_B08: i32 = 0;

// Data 1 aproveita seus caminhos atuais; data 2 fica em data/t2/.
const T1_B04: &str = "./data/B04.tif";
const T1_B08: &str = "./data/B08.tif";
const T1_SCL: &str = "./data/SCL.tif";
const T2_B04: &str = "./data/t2/B04.tif";
const T2_B08: &str = "./data/t2/B08.tif";
const T2_SCL: &str = "./data/t2/SCL.tif";
const OUT: &str = "./resultado_temporal";

struct Comparison {
    classes: Vec<u8>,
    loss: Vec<u8>,
    gain: Vec<u8>,
    valid_both: usize,
    stable_vegetation: usize,
    stable_nonvegetation: usize,
    loss_pixels: usize,
    gain_pixels: usize,
}

// Codigos: 0 = invalido, 1 = vegetacao estavel, 2 = perda,
//          3 = ganho, 4 = nao-vegetacao estavel.
fn compare_scenes(before: &SceneMasks, after: &SceneMasks) -> Comparison {
    assert_eq!(before.valid.len(), after.valid.len());
    assert_eq!(before.vegetation.len(), after.vegetation.len());
    assert_eq!(before.valid.len(), before.vegetation.len());
    assert_eq!(after.valid.len(), after.vegetation.len());
    let n = before.valid.len();
    let mut comparison = Comparison {
        classes: vec![0; n],
        loss: vec![0; n],
        gain: vec![0; n],
        valid_both: 0,
        stable_vegetation: 0,
        stable_nonvegetation: 0,
        loss_pixels: 0,
        gain_pixels: 0,
    };
    for i in 0..n {
        if before.valid[i] == 0 || after.valid[i] == 0 {
            continue;
        }
        comparison.valid_both += 1;
        comparison.classes[i] = match (before.vegetation[i], after.vegetation[i]) {
            (1, 1) => {
                comparison.stable_vegetation += 1;
                1
            }
            (1, 0) => {
                comparison.loss_pixels += 1;
                comparison.loss[i] = 1;
                2
            }
            (0, 1) => {
                comparison.gain_pixels += 1;
                comparison.gain[i] = 1;
                3
            }
            _ => {
                comparison.stable_nonvegetation += 1;
                4
            }
        };
    }
    comparison
}

fn same_float(x: f64, y: f64) -> bool {
    (x - y).abs() < 1e-5
}

fn check_scene_geo(
    b04_path: &str,
    b08_path: &str,
    scl_path: &str,
    row_offset: usize,
    col_offset: usize,
    size: usize,
) -> Result<UtmCrs, String> {
    let red =
        GeoTiffFile::open(b04_path).map_err(|error| format!("Erro no GeoTIFF B04: {error}"))?;
    let nir =
        GeoTiffFile::open(b08_path).map_err(|error| format!("Erro no GeoTIFF B08: {error}"))?;
    let scl =
        GeoTiffFile::open(scl_path).map_err(|error| format!("Erro no GeoTIFF SCL: {error}"))?;
    let epsg = red.epsg().ok_or("B04 sem codigo EPSG")?;
    let crs = utm_crs_from_epsg(epsg)?;
    if red.epsg() != nir.epsg() {
        return Err("EPSG B04/B08 diferente".into());
    }
    if red.epsg() != scl.epsg() {
        return Err("EPSG B04/SCL diferente".into());
    }
    if (red.width(), red.height()) != (nir.width(), nir.height()) {
        return Err("Tamanho B04/B08 diferente".into());
    }
    let a = red.transform().ok_or("B04 sem georreferenciamento")?;
    let b = nir.transform().ok_or("B08 sem georreferenciamento")?;
    let c = scl.transform().ok_or("SCL sem georreferenciamento")?;
    for (x, y) in [
        (a.origin_x, b.origin_x),
        (a.origin_y, b.origin_y),
        (a.pixel_width, b.pixel_width),
        (a.pixel_height, b.pixel_height),
        (a.skew_x, b.skew_x),
        (a.skew_y, b.skew_y),
        (a.origin_x, c.origin_x),
        (a.origin_y, c.origin_y),
        (a.pixel_width * 2.0, c.pixel_width),
        (a.pixel_height * 2.0, c.pixel_height),
        (a.skew_x * 2.0, c.skew_x),
        (a.skew_y * 2.0, c.skew_y),
    ] {
        if !same_float(x, y) {
            return Err("Grades de B04/B08/SCL desalinhadas".into());
        }
    }
    if red.width() != scl.width() * 2 || red.height() != scl.height() * 2 {
        return Err("SCL nao equivale a grade 20m esperada".into());
    }
    if row_offset % 2 != 0 || col_offset % 2 != 0 || size % 2 != 0 {
        return Err("Recorte precisa estar alinhado aos pixels da SCL de 20m".into());
    }
    if row_offset + size > red.height() as usize || col_offset + size > red.width() as usize {
        return Err("Recorte excede imagem".into());
    }
    Ok(crs)
}

fn check_temporal_grid(before_path: &str, after_path: &str) -> Result<f64, String> {
    let a = GeoTiffFile::open(before_path).map_err(|error| format!("Erro no B04 T1: {error}"))?;
    let b = GeoTiffFile::open(after_path).map_err(|error| format!("Erro no B04 T2: {error}"))?;
    if a.epsg() != b.epsg() {
        return Err("EPSG T1/T2 diferente; reprojecao automatica nao e realizada".into());
    }
    if (a.width(), a.height()) != (b.width(), b.height()) {
        return Err("Dimensoes T1/T2 diferentes; alinhe as cenas antes de comparar".into());
    }
    let x = a.transform().ok_or("B04 T1 sem georreferenciamento")?;
    let y = b.transform().ok_or("B04 T2 sem georreferenciamento")?;
    for (u, v) in [
        (x.origin_x, y.origin_x),
        (x.origin_y, y.origin_y),
        (x.pixel_width, y.pixel_width),
        (x.pixel_height, y.pixel_height),
        (x.skew_x, y.skew_x),
        (x.skew_y, y.skew_y),
    ] {
        if !same_float(u, v) {
            return Err("T1/T2 tem grades diferentes; alinhe antes de comparar".into());
        }
    }
    Ok((x.pixel_width * x.pixel_height - x.skew_x * x.skew_y).abs())
}

fn gray_png(data: &[u8], size: usize, path: &str) {
    let img = GrayImage::from_raw(size as u32, size as u32, data.to_vec()).unwrap();
    img.save(path).expect("Nao foi possivel salvar imagem");
}

fn rgb_png(data: &[u8], size: usize, path: &str) {
    let img = RgbImage::from_raw(size as u32, size as u32, data.to_vec()).unwrap();
    img.save(path).expect("Nao foi possivel salvar imagem RGB");
}

fn make_ndvi_image(scene: &SceneMasks, size: usize, path: &str) {
    let mut pixels = Vec::with_capacity(size * size * 3);
    for i in 0..size * size {
        let color = if scene.valid[i] == 0 {
            [76, 84, 97]
        } else {
            let v = scene.ndvi_scaled[i] as f32 / 1000.0;
            if v < 0.0 {
                [60, 105, 155]
            } else if v < 0.2 {
                [214, 180, 103]
            } else if v < 0.4 {
                [157, 184, 90]
            } else if v < 0.6 {
                [75, 153, 88]
            } else {
                [20, 106, 65]
            }
        };
        pixels.extend_from_slice(&color);
    }
    rgb_png(&pixels, size, path);
}

fn make_change_image(classes: &[u8], size: usize, path: &str) {
    let mut pixels = Vec::with_capacity(size * size * 3);
    for &c in classes {
        let rgb = match c {
            1 => [38, 143, 91],   // vegetacao estavel
            2 => [234, 70, 71],   // perda
            3 => [63, 154, 232],  // ganho
            4 => [171, 148, 103], // nao vegetado estavel
            _ => [84, 91, 103],   // invalido em ao menos uma data
        };
        pixels.extend_from_slice(&rgb);
    }
    rgb_png(&pixels, size, path);
}

fn make_loss_overlay(red_preview: &[u8], loss: &[u8], size: usize, path: &str) {
    let mut img = RgbImage::new(size as u32, size as u32);
    for i in 0..size * size {
        let g = red_preview[i];
        let color = if loss[i] == 1 {
            [241, 51, 64]
        } else {
            [g, g, g]
        };
        img.put_pixel((i % size) as u32, (i / size) as u32, Rgb(color));
    }
    img.save(path).unwrap();
}

// Gera todas as imagens sem mudar o processamento matematico.
// Painel de tres quadros, sem legendas sobre os pixels; nomes e legendas ficam no artigo.
fn export_comparison_strip() {
    let paths = [
        format!("{OUT}/t1_ndvi.png"),
        format!("{OUT}/t2_ndvi.png"),
        format!("{OUT}/mudancas.png"),
    ];
    let mut out = RgbImage::new((SIZE * 3) as u32, SIZE as u32);
    for (idx, path) in paths.iter().enumerate() {
        let img = image::open(path)
            .expect("Falha ao abrir PNG do painel")
            .to_rgb8();
        for y in 0..SIZE {
            for x in 0..SIZE {
                out.put_pixel(
                    (idx * SIZE + x) as u32,
                    y as u32,
                    *img.get_pixel(x as u32, y as u32),
                );
            }
        }
    }
    out.save(format!("{OUT}/figura_comparacao_ndvi.png"))
        .unwrap();
}

fn export_scene(prefix: &str, scene: &SceneMasks) {
    gray_png(&scene.red_preview, SIZE, &format!("{OUT}/{prefix}_b04.png"));
    gray_png(&scene.nir_preview, SIZE, &format!("{OUT}/{prefix}_b08.png"));
    make_ndvi_image(scene, SIZE, &format!("{OUT}/{prefix}_ndvi.png"));
    generate_grayimage(
        &scene.valid,
        SIZE as u32,
        &format!("{OUT}/{prefix}_valid.png"),
    );
    generate_grayimage(
        &scene.vegetation,
        SIZE as u32,
        &format!("{OUT}/{prefix}_vegetation.png"),
    );
}

// Calcula as areas de cada componente uma unica vez, agrupando as arestas
// por identificador do Flood Fill (sem varrer o raster para cada mancha).
fn component_areas_fast(labels: &[u32], count: u32, size: usize) -> Vec<ComponentArea> {
    assert_eq!(labels.len(), size * size);
    let mut groups: Vec<Vec<Edge>> = vec![Vec::new(); count as usize];
    let mut pixels = vec![0usize; count as usize];
    for r in 0..size {
        for c in 0..size {
            let id = labels[r * size + c];
            if id == 0 {
                continue;
            }
            let idx = (id - 1) as usize;
            pixels[idx] += 1;
            let edges = &mut groups[idx];
            if r == 0 || labels[(r - 1) * size + c] != id {
                edges.push(Edge {
                    start: Point { x: c, y: r },
                    end: Point { x: c + 1, y: r },
                });
            }
            if r + 1 == size || labels[(r + 1) * size + c] != id {
                edges.push(Edge {
                    start: Point { x: c + 1, y: r + 1 },
                    end: Point { x: c, y: r + 1 },
                });
            }
            if c == 0 || labels[r * size + c - 1] != id {
                edges.push(Edge {
                    start: Point { x: c, y: r + 1 },
                    end: Point { x: c, y: r },
                });
            }
            if c + 1 == size || labels[r * size + c + 1] != id {
                edges.push(Edge {
                    start: Point { x: c + 1, y: r },
                    end: Point { x: c + 1, y: r + 1 },
                });
            }
        }
    }
    groups
        .into_iter()
        .enumerate()
        .map(|(idx, edges)| {
            let contours = connect_all_edges(&edges);
            let area_pixels = calculate_total_area(&contours);
            assert_eq!(
                area_pixels,
                pixels[idx] as f64,
                "Erro geometrico na mancha {}",
                idx + 1
            );
            ComponentArea {
                label: (idx + 1) as u32,
                pixels: pixels[idx],
                contours: contours.len(),
                area_pixels,
            }
        })
        .collect()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn format_ha_pt(value: f64) -> String {
    format!("{value:.3}").replace('.', ",")
}

fn main() {
    if let Err(error) = run() {
        eprintln!("ERRO DE VALIDACAO: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let validate_only = args.first().is_some_and(|value| value == "--validate-only");
    if validate_only {
        args.remove(0);
    }
    // Caminhos por argumentos opcionais, mantendo a execucao direta como antes.
    let t1_b04 = args.get(0).map(String::as_str).unwrap_or(T1_B04);
    let t1_b08 = args.get(1).map(String::as_str).unwrap_or(T1_B08);
    let t1_scl = args.get(2).map(String::as_str).unwrap_or(T1_SCL);
    let t2_b04 = args.get(3).map(String::as_str).unwrap_or(T2_B04);
    let t2_b08 = args.get(4).map(String::as_str).unwrap_or(T2_B08);
    let t2_scl = args.get(5).map(String::as_str).unwrap_or(T2_SCL);
    let row_offset: usize = args
        .get(6)
        .map(|v| v.parse().map_err(|_| "Linha invalida"))
        .transpose()?
        .unwrap_or(ROW_OFFSET);
    let col_offset: usize = args
        .get(7)
        .map(|v| v.parse().map_err(|_| "Coluna invalida"))
        .transpose()?
        .unwrap_or(COL_OFFSET);
    let ndvi_threshold: i32 = args
        .get(8)
        .map(|v| v.parse().map_err(|_| "Limiar invalido"))
        .transpose()?
        .unwrap_or(NDVI_THRESHOLD);
    let offset_t1_red: i32 = args
        .get(9)
        .map(|v| v.parse().map_err(|_| "Offset B04 T1 invalido"))
        .transpose()?
        .unwrap_or(T1_OFFSET_B04);
    let offset_t1_nir: i32 = args
        .get(10)
        .map(|v| v.parse().map_err(|_| "Offset B08 T1 invalido"))
        .transpose()?
        .unwrap_or(T1_OFFSET_B08);
    let offset_t2_red: i32 = args
        .get(11)
        .map(|v| v.parse().map_err(|_| "Offset B04 T2 invalido"))
        .transpose()?
        .unwrap_or(T2_OFFSET_B04);
    let offset_t2_nir: i32 = args
        .get(12)
        .map(|v| v.parse().map_err(|_| "Offset B08 T2 invalido"))
        .transpose()?
        .unwrap_or(T2_OFFSET_B08);
    if !(-1000..=1000).contains(&ndvi_threshold) {
        return Err("NDVI fora de -1..1".into());
    }
    for p in [t1_b04, t1_b08, t1_scl, t2_b04, t2_b08, t2_scl] {
        if !std::path::Path::new(p).is_file() {
            return Err(format!("Arquivo ausente: {p}"));
        }
    }
    let crs_t1 = check_scene_geo(t1_b04, t1_b08, t1_scl, row_offset, col_offset, SIZE)?;
    let crs_t2 = check_scene_geo(t2_b04, t2_b08, t2_scl, row_offset, col_offset, SIZE)?;
    let pixel_area_m2 = check_temporal_grid(t1_b04, t2_b04)?;
    if crs_t1 != crs_t2 {
        return Err("CRS UTM T1/T2 diferente".into());
    }
    println!(
        "VALIDACAO OK: EPSG {}, fuso {}{}, area do pixel {:.4} m2",
        crs_t1.epsg,
        crs_t1.zone,
        if crs_t1.northern { "N" } else { "S" },
        pixel_area_m2
    );
    if validate_only {
        return Ok(());
    }
    fs::create_dir_all(OUT)
        .map_err(|error| format!("Nao foi possivel criar pasta de saida: {error}"))?;
    let scene1 = process_initial_scene(
        t1_b04,
        t1_b08,
        t1_scl,
        row_offset,
        col_offset,
        SIZE,
        ndvi_threshold,
        false,
        offset_t1_red,
        offset_t1_nir,
    );
    let scene2 = process_initial_scene(
        t2_b04,
        t2_b08,
        t2_scl,
        row_offset,
        col_offset,
        SIZE,
        ndvi_threshold,
        false,
        offset_t2_red,
        offset_t2_nir,
    );
    export_scene("t1", &scene1);
    export_scene("t2", &scene2);
    let comp = compare_scenes(&scene1, &scene2);
    assert!(
        comp.loss_pixels + comp.gain_pixels <= comp.valid_both,
        "Perdas e ganhos excedem pixels validos em ambas as datas"
    );
    let comparable_area = comp.valid_both as f64 * pixel_area_m2;
    let changed_area = (comp.loss_pixels + comp.gain_pixels) as f64 * pixel_area_m2;
    assert!(
        changed_area <= comparable_area,
        "Area alterada excede area valida comparavel"
    );
    make_change_image(&comp.classes, SIZE, &format!("{OUT}/mudancas.png"));
    export_comparison_strip();
    generate_grayimage(&comp.loss, SIZE as u32, &format!("{OUT}/perda.png"));
    generate_grayimage(&comp.gain, SIZE as u32, &format!("{OUT}/ganho.png"));
    make_loss_overlay(
        &scene2.red_preview,
        &comp.loss,
        SIZE,
        &format!("{OUT}/perda_sobre_b04.png"),
    );

    let (loss_labels, loss_count) = process_flood_fill(&comp.loss, SIZE, false);
    let edges = process_edge(&comp.loss, SIZE);
    generate_edges_image(&edges, SIZE, &format!("{OUT}/perda_contornos.png"));
    let contours = connect_all_edges(&edges);
    let area_geom = calculate_total_area(&contours);
    assert_eq!(
        area_geom, comp.loss_pixels as f64,
        "Area de perda nao confere"
    );
    let component_areas = component_areas_fast(&loss_labels, loss_count, SIZE);
    let t1_veg = scene1.vegetation.iter().filter(|&&x| x == 1).count();
    let t2_veg = scene2.vegetation.iter().filter(|&&x| x == 1).count();
    let to_ha = |n: usize| n as f64 * pixel_area_m2 / 10000.0;
    println!("\n=== COMPARACAO TEMPORAL ===");
    println!(
        "T1 vegetal em toda mascara valida: {} pixels ({:.3} ha)",
        t1_veg,
        to_ha(t1_veg)
    );
    println!(
        "T2 vegetal em toda mascara valida: {} pixels ({:.3} ha)",
        t2_veg,
        to_ha(t2_veg)
    );
    println!("Pixels validos nas duas datas: {}", comp.valid_both);
    println!(
        "Possivel perda: {} pixels, {:.3} ha, {} componentes",
        comp.loss_pixels,
        to_ha(comp.loss_pixels),
        loss_count
    );
    println!(
        "Possivel ganho: {} pixels, {:.3} ha",
        comp.gain_pixels,
        to_ha(comp.gain_pixels)
    );
    println!(
        "Area por determinantes (perda): {:.3} ha",
        area_geom * pixel_area_m2 / 10000.0
    );
    println!(
        "ATENCAO: mudanca NDVI nao confirma desmatamento. Verifique offset radiometrico e dados de referencia."
    );
    show_geographic_coordinates(t1_b04, SIZE, row_offset, col_offset, &contours);

    let mut csv = String::from("mancha,pixels,contornos,area_por_determinantes_m2,area_ha\n");
    for a in &component_areas {
        writeln!(
            &mut csv,
            "{},{},{},{:.4},{:.6}",
            a.label,
            a.pixels,
            a.contours,
            a.area_pixels * pixel_area_m2,
            a.area_pixels * pixel_area_m2 / 10000.0
        )
        .unwrap();
    }
    fs::write(format!("{OUT}/manchas_perda.csv"), csv).unwrap();
    let mut metrics = String::from("metrica,valor\n");
    for (key, val) in [
        ("pixels_validos_duas_datas", comp.valid_both),
        ("vegetacao_estavel", comp.stable_vegetation),
        ("nao_vegetacao_estavel", comp.stable_nonvegetation),
        ("possivel_perda_pixels", comp.loss_pixels),
        ("possivel_ganho_pixels", comp.gain_pixels),
        ("numero_manchas_perda", loss_count as usize),
        (
            "vegetacao_t1_entre_pixels_comparaveis",
            comp.stable_vegetation + comp.loss_pixels,
        ),
        (
            "vegetacao_t2_entre_pixels_comparaveis",
            comp.stable_vegetation + comp.gain_pixels,
        ),
    ] {
        writeln!(&mut metrics, "{key},{val}").unwrap();
    }
    fs::write(format!("{OUT}/metricas.csv"), metrics).unwrap();

    let mut html = include_str!("dashboard.html").to_string();
    let area_loss = format_ha_pt(to_ha(comp.loss_pixels));
    let area_gain = format_ha_pt(to_ha(comp.gain_pixels));
    let values = [
        ("{{LOSS_HA}}", area_loss.as_str()),
        ("{{GAIN_HA}}", area_gain.as_str()),
    ];
    for (key, value) in values {
        html = html.replace(key, value);
    }
    html = html.replace("{{LOSS_COMPONENTS}}", &loss_count.to_string());
    html = html.replace("{{VALID_BOTH}}", &comp.valid_both.to_string());
    html = html.replace("{{ROW_OFFSET}}", &row_offset.to_string());
    html = html.replace("{{COL_OFFSET}}", &col_offset.to_string());
    html = html.replace(
        "{{NDVI_THRESHOLD}}",
        &format!("{:.2}", ndvi_threshold as f64 / 1000.0),
    );
    html = html.replace("{{AREA_T1}}", &format_ha_pt(to_ha(t1_veg)));
    html = html.replace("{{AREA_T2}}", &format_ha_pt(to_ha(t2_veg)));
    html = html.replace("{{T1_PATH}}", &html_escape(t1_b04));
    html = html.replace("{{T2_PATH}}", &html_escape(t2_b04));
    fs::write(format!("{OUT}/index.html"), html).unwrap();
    println!("\nPainel: {OUT}/index.html");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flood_fill3x3() {
        let mask = vec![1, 1, 0, 0, 0, 0, 0, 1, 1];

        let (labels, count) = process_flood_fill(&mask, 3, false);

        assert_eq!(count, 2);
        assert_eq!(labels, vec![1, 1, 0, 0, 0, 0, 0, 2, 2]);
    }

    #[test]
    fn test_flood_fill5x5() {
        let mask = vec![
            0, 1, 1, 0, 1, 0, 1, 1, 0, 1, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 1, 1,
        ];

        let (labels, count) = process_flood_fill(&mask, 5, false);

        assert_eq!(count, 4);
        assert_eq!(
            labels,
            vec![
                0, 1, 1, 0, 2, 0, 1, 1, 0, 2, 0, 0, 0, 0, 2, 3, 3, 0, 0, 0, 0, 0, 0, 4, 4
            ]
        );
    }

    #[test]
    fn test_edge_1x1() {
        let mask = vec![1];

        let edges = process_edge(&mask, 1);

        assert_eq!(edges.len(), 4);

        assert_eq!(
            edges[0],
            Edge {
                start: Point { x: 0, y: 0 },
                end: Point { x: 1, y: 0 },
            }
        );

        assert_eq!(
            edges[1],
            Edge {
                start: Point { x: 1, y: 1 },
                end: Point { x: 0, y: 1 },
            }
        );

        assert_eq!(
            edges[2],
            Edge {
                start: Point { x: 0, y: 1 },
                end: Point { x: 0, y: 0 },
            }
        );

        assert_eq!(
            edges[3],
            Edge {
                start: Point { x: 1, y: 0 },
                end: Point { x: 1, y: 1 },
            }
        );
    }

    #[test]
    fn test_connect_edges_1x1() {
        let mask = vec![1];

        let edges = process_edge(&mask, 1);
        let contours = connect_all_edges(&edges);
        assert_eq!(contours.len(), 1);
        let polygon = &contours[0];

        assert_eq!(polygon.len(), 4);

        assert_eq!(
            polygon,
            &vec![
                Point { x: 0, y: 0 },
                Point { x: 1, y: 0 },
                Point { x: 1, y: 1 },
                Point { x: 0, y: 1 },
            ]
        );
    }

    #[test]
    fn test_connect_edges_l_shape() {
        let mask = vec![1, 1, 0, 1, 0, 0, 0, 0, 0];

        let edges = process_edge(&mask, 3);
        let contours = connect_all_edges(&edges);
        assert_eq!(contours.len(), 1);
        let polygon = &contours[0];

        assert_eq!(edges.len(), 8);
        assert_eq!(polygon.len(), 8);
    }

    #[test]
    fn test_polygon_area_l_shape() {
        let mask = vec![1, 1, 0, 1, 0, 0, 0, 0, 0];

        let edges = process_edge(&mask, 3);
        let contours = connect_all_edges(&edges);
        assert_eq!(contours.len(), 1);
        let polygon = &contours[0];
        let area = calculate_polygon_area(polygon);

        assert_eq!(area, 3.0);
    }

    #[test]
    fn test_polygon_area_square() {
        let mask = vec![1, 1, 1, 1];

        let edges = process_edge(&mask, 2);
        let contours = connect_all_edges(&edges);
        assert_eq!(contours.len(), 1);
        let polygon = &contours[0];
        let area = calculate_polygon_area(polygon);

        assert_eq!(area, 4.0);
    }

    #[test]
    fn test_edges_square_2x2() {
        let edges = process_edge(&[1, 1, 1, 1], 2);
        assert_eq!(edges.len(), 8);
    }

    #[test]
    fn test_multiple_contours() {
        let mask = vec![
            1, 1, 0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 0,
        ];
        let contours = connect_all_edges(&process_edge(&mask, 5));
        assert_eq!(contours.len(), 3);
        assert_eq!(calculate_total_area(&contours), 9.0);
    }

    #[test]
    fn test_contour_with_hole() {
        let mask = vec![1, 1, 1, 1, 0, 1, 1, 1, 1];
        let contours = connect_all_edges(&process_edge(&mask, 3));
        assert_eq!(contours.len(), 2);
        assert_eq!(calculate_total_area(&contours), 8.0);
        assert_eq!(
            contours
                .iter()
                .map(|c| calculate_signed_area(c).signum())
                .sum::<f64>(),
            0.0
        );
    }

    #[test]
    fn test_diagonal_pixels() {
        let mask = vec![1, 0, 0, 1];
        let contours = connect_all_edges(&process_edge(&mask, 2));
        assert_eq!(contours.len(), 2);
        assert_eq!(calculate_total_area(&contours), 2.0);
    }

    #[test]
    fn test_component_areas_three_spots() {
        let mask = vec![
            1, 1, 0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 0,
        ];
        let (labels, count) = process_flood_fill(&mask, 5, false);
        let areas = calculate_component_areas(&labels, count, 5);
        assert_eq!(areas.len(), 3);
        assert_eq!(
            areas.iter().map(|a| a.area_pixels).collect::<Vec<_>>(),
            vec![3.0, 2.0, 4.0]
        );
        assert_eq!(areas.iter().map(|a| a.area_pixels).sum::<f64>(), 9.0);
    }

    #[test]
    fn test_component_area_with_hole() {
        let mask = vec![1, 1, 1, 1, 0, 1, 1, 1, 1];
        let (labels, count) = process_flood_fill(&mask, 3, false);
        let areas = calculate_component_areas(&labels, count, 3);
        assert_eq!(areas.len(), 1);
        assert_eq!(areas[0].contours, 2);
        assert_eq!(areas[0].area_pixels, 8.0);
    }

    #[test]
    fn test_utm_to_latlon_southern_hemisphere() {
        // UTM fuso 20S: meridiano central -63 graus; 10 milhoes = equador.
        let crs = utm_crs_from_epsg(32720).unwrap();
        let (lat, lon) = utm_to_latlon(500_000.0, 10_000_000.0, crs);
        assert!(lat.abs() < 1e-5);
        assert!((lon + 63.0).abs() < 1e-5);
    }

    #[test]
    fn test_recognizes_utm_zones_20_and_21_south() {
        assert_eq!(
            utm_crs_from_epsg(32720).unwrap(),
            UtmCrs {
                epsg: 32720,
                zone: 20,
                northern: false
            }
        );
        assert_eq!(
            utm_crs_from_epsg(32721).unwrap(),
            UtmCrs {
                epsg: 32721,
                zone: 21,
                northern: false
            }
        );
        assert!(utm_crs_from_epsg(4326).is_err());
    }

    #[test]
    fn test_utm_zone_21_central_meridian() {
        let crs = utm_crs_from_epsg(32721).unwrap();
        let (lat, lon) = utm_to_latlon(500_000.0, 10_000_000.0, crs);
        assert!(lat.abs() < 1e-5);
        assert!((lon + 57.0).abs() < 1e-5);
    }

    #[test]
    fn test_utm_real_reference_point() {
        // Coordenadas de referencia independentes WGS84 para (372000, 9878040) no EPSG:32720.
        let crs = utm_crs_from_epsg(32720).unwrap();
        let (lat, lon) = utm_to_latlon(372_000.0, 9_878_040.0, crs);
        assert!((lat - (-1.1031851459)).abs() < 0.000001);
        assert!((lon - (-64.1504377400)).abs() < 0.000001);
    }

    #[test]
    fn test_empty_mask() {
        let edges = process_edge(&[0, 0, 0, 0], 2);
        let contours = connect_all_edges(&edges);
        assert!(contours.is_empty());
        assert_eq!(calculate_total_area(&contours), 0.0);
    }

    #[test]
    fn test_compare_scenes_with_invalid() {
        let mk = |valid: Vec<u8>, vegetation: Vec<u8>| SceneMasks {
            ndvi_scaled: vec![0; valid.len()],
            red_preview: vec![0; valid.len()],
            nir_preview: vec![0; valid.len()],
            valid,
            vegetation,
        };
        let t1 = mk(vec![1, 1, 1, 1, 0, 1], vec![1, 0, 1, 0, 1, 1]);
        let t2 = mk(vec![1, 1, 1, 1, 1, 0], vec![0, 1, 1, 0, 0, 0]);
        let c = compare_scenes(&t1, &t2);
        assert_eq!(c.valid_both, 4);
        assert_eq!(c.loss_pixels, 1);
        assert_eq!(c.gain_pixels, 1);
        assert_eq!(c.stable_vegetation, 1);
        assert_eq!(c.stable_nonvegetation, 1);
        assert_eq!(c.classes, vec![2, 3, 1, 4, 0, 0]);
    }

    #[test]
    fn test_identical_scenes_have_no_loss_or_gain() {
        let scene = SceneMasks {
            valid: vec![1, 1, 0, 1, 1, 1],
            vegetation: vec![1, 0, 1, 1, 0, 1],
            ndvi_scaled: vec![600, 200, i16::MIN, 450, 100, 700],
            red_preview: vec![10; 6],
            nir_preview: vec![20; 6],
        };

        let comparison = compare_scenes(&scene, &scene);

        assert_eq!(comparison.valid_both, 5);
        assert_eq!(comparison.loss_pixels, 0);
        assert_eq!(comparison.gain_pixels, 0);
        assert!(comparison.loss.iter().all(|&value| value == 0));
        assert!(comparison.gain.iter().all(|&value| value == 0));
        assert_eq!(comparison.classes, vec![1, 4, 0, 1, 4, 1]);
    }

    #[test]
    fn test_fast_component_area_with_hole() {
        let mask = vec![1, 1, 1, 1, 0, 1, 1, 1, 1];
        let (labels, count) = process_flood_fill(&mask, 3, false);
        let areas = component_areas_fast(&labels, count, 3);
        assert_eq!(areas.len(), 1);
        assert_eq!(areas[0].contours, 2);
        assert_eq!(areas[0].area_pixels, 8.0);
    }

    #[test]
    fn test_fast_component_area_diagonal() {
        let mask = vec![1, 0, 0, 1];
        let (labels, count) = process_flood_fill(&mask, 2, false);
        let areas = component_areas_fast(&labels, count, 2);
        assert_eq!(areas.len(), 2);
        assert_eq!(
            areas.iter().map(|a| a.area_pixels).collect::<Vec<_>>(),
            vec![1.0, 1.0]
        );
    }
}
