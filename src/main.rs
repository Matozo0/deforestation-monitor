use geotiff_reader::GeoTiffFile;
use image::{GrayImage, Luma};
use std::collections::{HashMap, VecDeque};
use tiff_reader::TiffFile;

struct SceneMasks {
    valid: Vec<u8>,
    vegetation: Vec<u8>,
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

    let mut vegetation_count: usize = 0;
    let mut non_vegetation_count: usize = 0;
    let mut invalid_count: usize = 0;

    let mut min_ndvi = 1000;
    let mut max_ndvi = -1000;

    // Itera pelo B04, B08 e SCL para gerar a mascara valida e mascara de vegetação
    for i in 0..size {
        for j in 0..size {
            let index = i * size + j;

            let red = tile_b04[[i, j]] as i32;
            let nir = tile_b08[[i, j]] as i32;

            let scl_value = tile_scl[[i / scale, j / scale]];
            let valid = matches!(scl_value, 4 | 5); // Rejeita tudo menos os 4 (vegetação) e 5 (não vegetação)

            if !valid {
                invalid_count += 1;
                continue;
            }

            let denominator = nir + red;

            if denominator == 0 {
                invalid_count += 1;
                continue;
            }

            valid_mask[index] = 1;

            let ndvi = ((nir - red) * 1000) / denominator;

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
    }
}

fn process_flood_fill(mask: &[u8], size: usize, _debug: bool) -> (Vec<u32>, u32) {
    process_flood_fill_rect(mask, size, size)
}

fn process_flood_fill_rect(mask: &[u8], width: usize, height: usize) -> (Vec<u32>, u32) {
    assert_eq!(mask.len(), width * height);
    let mut labels = vec![0u32; mask.len()];
    let mut current_label = 0u32;
    let mut max_queue = 0usize;

    let directions = [
        (-1, 0), // cima
        (1, 0),  // baixo
        (0, -1), // esquerda
        (0, 1),  // direita
    ];

    for row in 0..height {
        for col in 0..width {
            let index = row * width + col;

            if mask[index] == 0 || labels[index] != 0 {
                continue;
            }

            current_label += 1;

            let mut queue = VecDeque::new();
            labels[index] = current_label;
            queue.push_back((row, col));
            max_queue = max_queue.max(queue.len());

            while let Some((r, c)) = queue.pop_front() {
                for (dr, dc) in directions {
                    let nr = r as isize + dr;
                    let nc = c as isize + dc;

                    if nr < 0 || nc < 0 || nr >= height as isize || nc >= width as isize {
                        continue;
                    }

                    let nr = nr as usize;
                    let nc = nc as usize;
                    let nindex = nr * width + nc;

                    if mask[nindex] == 1 && labels[nindex] == 0 {
                        labels[nindex] = current_label;

                        queue.push_back((nr, nc));
                        max_queue = max_queue.max(queue.len());
                    }
                }
            }
        }
    }
    if mask.len() > 512 * 512 {
        println!("Maior fila observada no Flood Fill global: {max_queue} pixels");
    }
    (labels, current_label)
}

fn process_edge(mask: &[u8], size: usize) -> Vec<Edge> {
    process_edge_rect(mask, size, size)
}

fn process_edge_rect(mask: &[u8], width: usize, height: usize) -> Vec<Edge> {
    assert_eq!(mask.len(), width * height);
    let mut edges: Vec<Edge> = Vec::new();

    for i in 0..height {
        for j in 0..width {
            let index = i * width + j;

            if mask[index] == 0 {
                continue;
            }

            // Aresta cima
            if i == 0 || mask[(i - 1) * width + j] == 0 {
                let edge = Edge {
                    start: Point { x: j, y: i },
                    end: Point { x: j + 1, y: i },
                };
                edges.push(edge);
            }
            // Aresta baixo
            if i + 1 == height || mask[(i + 1) * width + j] == 0 {
                let edge = Edge {
                    start: Point { x: j + 1, y: i + 1 },
                    end: Point { x: j, y: i + 1 },
                };
                edges.push(edge);
            }
            // Aresta esquerda
            if j == 0 || mask[i * width + (j - 1)] == 0 {
                let edge = Edge {
                    start: Point { x: j, y: i + 1 },
                    end: Point { x: j, y: i },
                };
                edges.push(edge);
            }
            // Aresta direita
            if j + 1 == width || mask[i * width + (j + 1)] == 0 {
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

fn component_areas_fast_rect(
    labels: &[u32],
    count: u32,
    width: usize,
    height: usize,
) -> Vec<ComponentArea> {
    assert_eq!(labels.len(), width * height);
    let mut groups: Vec<Vec<Edge>> = vec![Vec::new(); count as usize];
    let mut pixels = vec![0usize; count as usize];
    for row in 0..height {
        for col in 0..width {
            let label = labels[row * width + col];
            if label == 0 {
                continue;
            }
            let index = (label - 1) as usize;
            pixels[index] += 1;
            let edges = &mut groups[index];
            if row == 0 || labels[(row - 1) * width + col] != label {
                edges.push(Edge {
                    start: Point { x: col, y: row },
                    end: Point { x: col + 1, y: row },
                });
            }
            if row + 1 == height || labels[(row + 1) * width + col] != label {
                edges.push(Edge {
                    start: Point {
                        x: col + 1,
                        y: row + 1,
                    },
                    end: Point { x: col, y: row + 1 },
                });
            }
            if col == 0 || labels[row * width + col - 1] != label {
                edges.push(Edge {
                    start: Point { x: col, y: row + 1 },
                    end: Point { x: col, y: row },
                });
            }
            if col + 1 == width || labels[row * width + col + 1] != label {
                edges.push(Edge {
                    start: Point { x: col + 1, y: row },
                    end: Point {
                        x: col + 1,
                        y: row + 1,
                    },
                });
            }
        }
    }
    groups
        .into_iter()
        .enumerate()
        .map(|(index, edges)| {
            let contours = connect_all_edges(&edges);
            let area_pixels = calculate_total_area(&contours);
            assert_eq!(
                area_pixels,
                pixels[index] as f64,
                "Erro geometrico na mancha {}",
                index + 1
            );
            ComponentArea {
                label: (index + 1) as u32,
                pixels: pixels[index],
                contours: contours.len(),
                area_pixels,
            }
        })
        .collect()
}

// Transformacao afim do GeoTIFF B04 fornecido (EPSG:32720).
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
        _ => return Err(format!("EPSG {epsg} nao suportado: esperado WGS84 / UTM")),
    };
    Ok(UtmCrs {
        epsg,
        zone,
        northern,
    })
}

// Inversa manual da projecao UTM / Transversa de Mercator no WGS84.
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
    width: usize,
    height: usize,
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
        ("Canto superior direito", Point { x: width, y: 0 }),
        ("Canto inferior esquerdo", Point { x: 0, y: height }),
        (
            "Canto inferior direito",
            Point {
                x: width,
                y: height,
            },
        ),
        (
            "Centro aproximado",
            Point {
                x: width / 2,
                y: height / 2,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Block {
    row: usize,
    col: usize,
    width: usize,
    height: usize,
}

fn blocks(width: usize, height: usize, block_size: usize) -> Vec<Block> {
    assert!(block_size > 0);
    let mut result = Vec::new();
    for row in (0..height).step_by(block_size) {
        for col in (0..width).step_by(block_size) {
            result.push(Block {
                row,
                col,
                width: (width - col).min(block_size),
                height: (height - row).min(block_size),
            });
        }
    }
    result
}

fn copy_block_to_global(global: &mut [u8], global_width: usize, block: Block, local: &[u8]) {
    assert_eq!(local.len(), block.width * block.height);
    for row in 0..block.height {
        for col in 0..block.width {
            let global_index = (block.row + row) * global_width + block.col + col;
            global[global_index] = local[row * block.width + col];
        }
    }
}

fn same_float(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-5
}

fn validate_scene(
    b04_path: &str,
    b08_path: &str,
    scl_path: &str,
) -> Result<(usize, usize, f64, UtmCrs), String> {
    let red = GeoTiffFile::open(b04_path).map_err(|error| format!("Erro no B04: {error}"))?;
    let nir = GeoTiffFile::open(b08_path).map_err(|error| format!("Erro no B08: {error}"))?;
    let scl = GeoTiffFile::open(scl_path).map_err(|error| format!("Erro no SCL: {error}"))?;
    let epsg = red.epsg().ok_or("B04 sem EPSG")?;
    let crs = utm_crs_from_epsg(epsg)?;
    if red.epsg() != nir.epsg() || red.epsg() != scl.epsg() {
        return Err("EPSG B04/B08/SCL diferente".into());
    }
    if (red.width(), red.height()) != (nir.width(), nir.height()) {
        return Err("Dimensoes B04/B08 diferentes".into());
    }
    if red.width() != scl.width() * 2 || red.height() != scl.height() * 2 {
        return Err("SCL nao corresponde a grade de 20 m".into());
    }
    let a = red.transform().ok_or("B04 sem georreferenciamento")?;
    let b = nir.transform().ok_or("B08 sem georreferenciamento")?;
    let c = scl.transform().ok_or("SCL sem georreferenciamento")?;
    for (left, right) in [
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
        if !same_float(left, right) {
            return Err("Grades B04/B08/SCL desalinhadas".into());
        }
    }
    let pixel_area = (a.pixel_width * a.pixel_height - a.skew_x * a.skew_y).abs();
    Ok((red.width() as usize, red.height() as usize, pixel_area, crs))
}

struct SceneReaders {
    b04: TiffFile,
    b08: TiffFile,
    scl: TiffFile,
}

impl SceneReaders {
    fn open(b04_path: &str, b08_path: &str, scl_path: &str) -> Result<Self, String> {
        Ok(Self {
            b04: TiffFile::open(b04_path).map_err(|error| format!("Erro abrindo B04: {error}"))?,
            b08: TiffFile::open(b08_path).map_err(|error| format!("Erro abrindo B08: {error}"))?,
            scl: TiffFile::open(scl_path).map_err(|error| format!("Erro abrindo SCL: {error}"))?,
        })
    }
}

fn process_scene_block(
    readers: &SceneReaders,
    row_offset: usize,
    col_offset: usize,
    width: usize,
    height: usize,
    threshold: i32,
    offset_red: i32,
    offset_nir: i32,
) -> Result<Vec<u8>, String> {
    if row_offset % 2 != 0 || col_offset % 2 != 0 {
        return Err("Janela desalinhada da SCL de 20 m".into());
    }
    let red: ndarray::ArrayD<u16> = readers
        .b04
        .read_window(0, row_offset, col_offset, height, width)
        .map_err(|error| format!("Erro lendo B04: {error}"))?;
    let nir: ndarray::ArrayD<u16> = readers
        .b08
        .read_window(0, row_offset, col_offset, height, width)
        .map_err(|error| format!("Erro lendo B08: {error}"))?;
    let classes: ndarray::ArrayD<u8> = readers
        .scl
        .read_window(
            0,
            row_offset / 2,
            col_offset / 2,
            height.div_ceil(2),
            width.div_ceil(2),
        )
        .map_err(|error| format!("Erro lendo SCL: {error}"))?;
    let mut vegetation = vec![0u8; width * height];
    for row in 0..height {
        for col in 0..width {
            let red_dn = red[[row, col]] as i32;
            let nir_dn = nir[[row, col]] as i32;
            if red_dn == 0 || nir_dn == 0 || !matches!(classes[[row / 2, col / 2]], 4 | 5) {
                continue;
            }
            let red_value = red_dn + offset_red;
            let nir_value = nir_dn + offset_nir;
            let denominator = red_value + nir_value;
            if denominator > 0 && ((nir_value - red_value) * 1000) / denominator >= threshold {
                vegetation[row * width + col] = 1;
            }
        }
    }
    Ok(vegetation)
}

fn run_full_scene(
    b04: &str,
    b08: &str,
    scl: &str,
    threshold: i32,
    offset_red: i32,
    offset_nir: i32,
    validate_only: bool,
) -> Result<(), String> {
    let (width, height, pixel_area, crs) = validate_scene(b04, b08, scl)?;
    let pixel_count = width
        .checked_mul(height)
        .ok_or("Dimensoes excedem a plataforma")?;
    println!(
        "PROCESSAMENTO GLOBAL: EPSG {}, {} x {} pixels",
        crs.epsg, width, height
    );
    println!(
        "Memoria minima estimada: mascara {:.1} MiB; rotulos {:.1} MiB; fila e contornos adicionais variam",
        pixel_count as f64 / 1_048_576.0,
        pixel_count as f64 * 4.0 / 1_048_576.0
    );
    if validate_only {
        return Ok(());
    }
    let mut vegetation = vec![0u8; pixel_count];
    let all_blocks = blocks(width, height, 512);
    let readers = SceneReaders::open(b04, b08, scl)?;
    for (number, block) in all_blocks.iter().copied().enumerate() {
        let local = process_scene_block(
            &readers,
            block.row,
            block.col,
            block.width,
            block.height,
            threshold,
            offset_red,
            offset_nir,
        )?;
        copy_block_to_global(&mut vegetation, width, block, &local);
        if number + 1 == all_blocks.len() || block.col + block.width == width {
            println!("Progresso: {}/{} blocos", number + 1, all_blocks.len());
        }
    }
    let pixel_total = vegetation.iter().filter(|&&value| value == 1).count();
    let (labels, count) = process_flood_fill_rect(&vegetation, width, height);
    let edges = process_edge_rect(&vegetation, width, height);
    let contours = connect_all_edges(&edges);
    let area_pixels = calculate_total_area(&contours);
    assert_eq!(
        area_pixels, pixel_total as f64,
        "Area por determinantes diferente da contagem global"
    );
    show_geographic_coordinates(b04, width, height, 0, 0, &contours);
    drop(contours);
    drop(edges);
    let areas = component_areas_fast_rect(&labels, count, width, height);
    let sum: f64 = areas.iter().map(|area| area.area_pixels).sum();
    assert_eq!(sum, area_pixels);
    println!("Componentes globais: {count}");
    println!(
        "Area por contagem: {:.3} ha",
        pixel_total as f64 * pixel_area / 10_000.0
    );
    println!(
        "Area por determinantes: {:.3} ha",
        area_pixels * pixel_area / 10_000.0
    );
    Ok(())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut full_scene = false;
    let mut validate_only = false;
    loop {
        match args.first().map(String::as_str) {
            Some("--full") => {
                full_scene = true;
                args.remove(0);
            }
            Some("--validate-only") => {
                validate_only = true;
                args.remove(0);
            }
            _ => break,
        }
    }
    if full_scene {
        let b04 = args.first().map(String::as_str).unwrap_or("./data/B04.tif");
        let b08 = args.get(1).map(String::as_str).unwrap_or("./data/B08.tif");
        let scl = args.get(2).map(String::as_str).unwrap_or("./data/SCL.tif");
        let threshold = args
            .get(3)
            .map(|value| value.parse().expect("Limiar invalido"))
            .unwrap_or(400);
        let offset_red = args
            .get(4)
            .map(|value| value.parse().expect("Offset B04 invalido"))
            .unwrap_or(0);
        let offset_nir = args
            .get(5)
            .map(|value| value.parse().expect("Offset B08 invalido"))
            .unwrap_or(0);
        if let Err(error) = run_full_scene(
            b04,
            b08,
            scl,
            threshold,
            offset_red,
            offset_nir,
            validate_only,
        ) {
            eprintln!("ERRO DE VALIDACAO: {error}");
            std::process::exit(2);
        }
        return;
    }
    let size = 512;
    let row_offset = 2200;
    let col_offset = 7200;
    let scene = process_initial_scene(
        "./data/B04.tif",
        "./data/B08.tif",
        "./data/SCL.tif",
        row_offset,
        col_offset,
        size,
        400,
        false,
    );

    let (labels, count) = process_flood_fill(&scene.vegetation, size, false);
    println!("Componentes encontrados: {count}");

    let edges = process_edge(&scene.vegetation, size);
    println!("Arestas encontradas: {}", edges.len());
    generate_edges_image(&edges, size, "./data/edges.png");

    let contours = connect_all_edges(&edges);
    println!("Contornos encontrados: {}", contours.len());
    show_geographic_coordinates(
        "./data/B04.tif",
        size,
        size,
        row_offset,
        col_offset,
        &contours,
    );

    let area_pixels = calculate_total_area(&contours);
    let pixel_count = scene.vegetation.iter().filter(|&&v| v == 1).count();
    println!("Area por determinantes: {area_pixels} pixels²");
    println!("Area por contagem: {pixel_count} pixels²");
    println!("Area projetada aproximada: {:.4} ha", area_pixels / 100.0);

    assert_eq!(
        area_pixels, pixel_count as f64,
        "A area geometrica nao coincide com a contagem de pixels"
    );

    // Calculo independente para cada identificador do Flood Fill.
    let areas = calculate_component_areas(&labels, count, size);
    println!("\nArea por componente:");
    for component in &areas {
        println!(
            "Mancha {}: {} pixels; {} contornos; {:.4} ha",
            component.label,
            component.pixels,
            component.contours,
            component.area_pixels / 100.0
        );
    }
    let sum: f64 = areas.iter().map(|item| item.area_pixels).sum();
    assert_eq!(
        sum, area_pixels,
        "Soma das componentes diferente da area total"
    );
    println!("Total das {} manchas: {:.4} ha", areas.len(), sum / 100.0);
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
        let (lat, lon) = utm_to_latlon(500_000.0, 10_000_000.0, utm_crs_from_epsg(32720).unwrap());
        assert!(lat.abs() < 1e-5);
        assert!((lon + 63.0).abs() < 1e-5);
    }

    #[test]
    fn test_utm_real_reference_point() {
        // Coordenadas de referencia independentes WGS84 para (372000, 9878040) no EPSG:32720.
        let (lat, lon) = utm_to_latlon(372_000.0, 9_878_040.0, utm_crs_from_epsg(32720).unwrap());
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
    fn test_global_single_scene_is_independent_of_block_size() {
        let width = 525;
        let height = 519;
        let mut mask = vec![0u8; width * height];
        for row in 50..470 {
            for col in 40..490 {
                mask[row * width + col] = 1;
            }
        }
        for row in 200..260 {
            for col in 200..300 {
                mask[row * width + col] = 0;
            }
        }
        let expected_pixels = mask.iter().filter(|&&value| value == 1).count();
        for block_size in [64, 128, 256, 512] {
            let mut rebuilt = vec![0u8; width * height];
            for block in blocks(width, height, block_size) {
                let mut local = vec![0u8; block.width * block.height];
                for row in 0..block.height {
                    for col in 0..block.width {
                        local[row * block.width + col] =
                            mask[(block.row + row) * width + block.col + col];
                    }
                }
                copy_block_to_global(&mut rebuilt, width, block, &local);
            }
            assert_eq!(rebuilt, mask);
            let (labels, count) = process_flood_fill_rect(&rebuilt, width, height);
            let contours = connect_all_edges(&process_edge_rect(&rebuilt, width, height));
            assert_eq!(calculate_total_area(&contours), expected_pixels as f64);
            assert_eq!(
                component_areas_fast_rect(&labels, count, width, height)
                    .iter()
                    .map(|area| area.area_pixels)
                    .sum::<f64>(),
                expected_pixels as f64
            );
        }
    }
}
