use std::collections::VecDeque;

use image::GrayImage;
use tiff_reader::TiffFile;

// ============================================================
// ESTRUTURAS SIMPLES
// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Point {
    x: i64,
    y: i64,
}

#[derive(Clone, Copy, Debug)]
struct Edge {
    start: Point,
    end: Point,
}

struct SceneMasks {
    valid: Vec<u8>,
    vegetation: Vec<u8>,
}

// ============================================================
// SALVAR MÁSCARA COMO PNG
// ============================================================

fn generate_grayimage(mask: &[u8], size: u32, path: &str) {
    let pixels: Vec<u8> = mask.iter().map(|&v| if v == 1 { 255 } else { 0 }).collect();

    let img = GrayImage::from_raw(size, size, pixels).unwrap();

    img.save(path).unwrap();
}

// ============================================================
// PROCESSAR A CENA INICIAL
// ============================================================

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

    let mut vegetation_count = 0usize;
    let mut non_vegetation_count = 0usize;
    let mut invalid_count = 0usize;

    let mut min_ndvi = 1000;
    let mut max_ndvi = -1000;

    for i in 0..size {
        for j in 0..size {
            let index = i * size + j;

            let red = tile_b04[[i, j]] as i32;
            let nir = tile_b08[[i, j]] as i32;

            let scl_value = tile_scl[[i / scale, j / scale]];

            // SCL:
            // 4 = vegetação
            // 5 = não vegetado
            //
            // Água, nuvem, sombra etc. são descartados.

            let valid = matches!(scl_value, 4 | 5);

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

            // NDVI x1000
            let ndvi = ((nir - red) * 1000) / denominator;

            if ndvi >= threshold {
                vegetation_mask[index] = 1;
                vegetation_count += 1;
            } else {
                non_vegetation_count += 1;
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

// ============================================================
// ÁREA ABERTA DA CENA ATUAL
//
// IMPORTANTE:
// isso ainda NÃO significa desmatamento.
//
// É apenas:
// pixel válido + não vegetação.
//
// Depois vamos trocar pela diferença entre t0 e t1.
// ============================================================

fn generate_open_mask(scene: &SceneMasks) -> Vec<u8> {
    let mut mask = vec![0u8; scene.valid.len()];

    for i in 0..mask.len() {
        if scene.valid[i] == 1 && scene.vegetation[i] == 0 {
            mask[i] = 1;
        }
    }

    mask
}

// ============================================================
// FLOOD FILL
//
// 0 = fundo
// 1 = mancha 1
// 2 = mancha 2
// ...
// ============================================================

fn process_flood_fill(mask: &[u8], size: usize) -> (Vec<u32>, u32) {
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

// ============================================================
// MÁSCARA VISUAL DAS BORDAS
// ============================================================

fn generate_border_mask(labels: &[u32], size: usize) -> Vec<u8> {
    let mut border = vec![0u8; labels.len()];

    for row in 0..size {
        for col in 0..size {
            let index = row * size + col;

            let label = labels[index];

            if label == 0 {
                continue;
            }

            let top = row > 0 && labels[(row - 1) * size + col] == label;

            let bottom = row + 1 < size && labels[(row + 1) * size + col] == label;

            let left = col > 0 && labels[row * size + col - 1] == label;

            let right = col + 1 < size && labels[row * size + col + 1] == label;

            if !top || !bottom || !left || !right {
                border[index] = 1;
            }
        }
    }

    border
}

// ============================================================
// GERAR AS ARESTAS DE UMA MANCHA
//
// Aqui usamos os CANTOS dos pixels.
// ============================================================

fn process_border(labels: &[u32], size: usize, label: u32) -> Vec<Edge> {
    let mut edges = Vec::new();

    for row in 0..size {
        for col in 0..size {
            let index = row * size + col;

            if labels[index] != label {
                continue;
            }

            let x = col as i64;
            let y = row as i64;

            // CIMA
            if row == 0 || labels[(row - 1) * size + col] != label {
                edges.push(Edge {
                    start: Point { x, y },

                    end: Point { x: x + 1, y },
                });
            }

            // DIREITA
            if col + 1 == size || labels[row * size + col + 1] != label {
                edges.push(Edge {
                    start: Point { x: x + 1, y },

                    end: Point { x: x + 1, y: y + 1 },
                });
            }

            // BAIXO
            if row + 1 == size || labels[(row + 1) * size + col] != label {
                edges.push(Edge {
                    start: Point { x: x + 1, y: y + 1 },

                    end: Point { x, y: y + 1 },
                });
            }

            // ESQUERDA
            if col == 0 || labels[row * size + col - 1] != label {
                edges.push(Edge {
                    start: Point { x, y: y + 1 },

                    end: Point { x, y },
                });
            }
        }
    }

    edges
}

// ============================================================
// ORDENAR AS ARESTAS E FORMAR O POLÍGONO
//
// Versão simples:
// assume uma mancha sem furos.
// ============================================================

fn generate_polygon(edges: &[Edge]) -> Option<Vec<Point>> {
    if edges.is_empty() {
        return None;
    }

    let mut used = vec![false; edges.len()];

    let first_point = edges[0].start;

    let mut current = edges[0].end;

    used[0] = true;

    let mut polygon = vec![first_point];

    while current != first_point {
        polygon.push(current);

        let mut next_edge = None;

        for i in 0..edges.len() {
            if !used[i] && edges[i].start == current {
                next_edge = Some(i);
                break;
            }
        }

        let index = next_edge?;

        used[index] = true;

        current = edges[index].end;

        // segurança contra loop inválido
        if polygon.len() > edges.len() + 1 {
            return None;
        }
    }

    // Se sobraram arestas,
    // provavelmente existe furo ou alguma
    // topologia mais complicada.
    if used.iter().any(|&v| !v) {
        return None;
    }

    Some(polygon)
}

// ============================================================
// DETERMINANTE / PRODUTO VETORIAL
// ============================================================

fn determinant(a: Point, b: Point, c: Point) -> i128 {
    let ab_x = (b.x - a.x) as i128;

    let ab_y = (b.y - a.y) as i128;

    let ac_x = (c.x - a.x) as i128;

    let ac_y = (c.y - a.y) as i128;

    ab_x * ac_y - ab_y * ac_x
}

// ============================================================
// REMOVER PONTOS COLINEARES
//
// Exemplo:
//
// A ---- B ---- C
//
// vira:
//
// A ----------- C
// ============================================================

fn simplify_polygon(polygon: &[Point]) -> Vec<Point> {
    if polygon.len() <= 3 {
        return polygon.to_vec();
    }

    let mut result = Vec::new();

    let n = polygon.len();

    for i in 0..n {
        let previous = polygon[(i + n - 1) % n];

        let current = polygon[i];

        let next = polygon[(i + 1) % n];

        if determinant(previous, current, next) != 0 {
            result.push(current);
        }
    }

    result
}

// ============================================================
// ÁREA ORIENTADA DO POLÍGONO
//
// Usada aqui apenas para saber
// a orientação dos vértices.
// ============================================================

fn polygon_area2(polygon: &[Point]) -> i128 {
    let mut area = 0i128;

    for i in 0..polygon.len() {
        let a = polygon[i];

        let b = polygon[(i + 1) % polygon.len()];

        area += a.x as i128 * b.y as i128 - b.x as i128 * a.y as i128;
    }

    area
}

// ============================================================
// TESTAR SE UM PONTO ESTÁ DENTRO DO TRIÂNGULO
// ============================================================

fn point_inside_triangle(point: Point, a: Point, b: Point, c: Point) -> bool {
    let d1 = determinant(a, b, point);

    let d2 = determinant(b, c, point);

    let d3 = determinant(c, a, point);

    d1 >= 0 && d2 >= 0 && d3 >= 0
}

// ============================================================
// EAR CLIPPING
//
// Divide o polígono em triângulos.
// ============================================================

fn triangulate(polygon: &[Point]) -> Option<Vec<[Point; 3]>> {
    if polygon.len() < 3 {
        return None;
    }

    let mut vertices = polygon.to_vec();

    // Padronizar orientação
    if polygon_area2(&vertices) < 0 {
        vertices.reverse();
    }

    let mut triangles = Vec::new();

    while vertices.len() > 3 {
        let n = vertices.len();

        let mut ear_found = false;

        for i in 0..n {
            let previous = vertices[(i + n - 1) % n];

            let current = vertices[i];

            let next = vertices[(i + 1) % n];

            // precisa ser convexo
            if determinant(previous, current, next) <= 0 {
                continue;
            }

            let mut contains_point = false;

            for j in 0..n {
                if j == i || j == (i + n - 1) % n || j == (i + 1) % n {
                    continue;
                }

                if point_inside_triangle(vertices[j], previous, current, next) {
                    contains_point = true;
                    break;
                }
            }

            if contains_point {
                continue;
            }

            triangles.push([previous, current, next]);

            vertices.remove(i);

            ear_found = true;

            break;
        }

        if !ear_found {
            return None;
        }
    }

    triangles.push([vertices[0], vertices[1], vertices[2]]);

    Some(triangles)
}

// ============================================================
// CALCULAR ÁREA E CENTROIDE
//
// Tudo vem dos triângulos.
//
// Sentinel-2 = 10 m por pixel.
// ============================================================

fn calculate_area_centroid(triangles: &[[Point; 3]]) -> Option<(i128, f64, f64)> {
    let mut total_area2 = 0i128;

    let mut centroid_x_sum = 0i128;

    let mut centroid_y_sum = 0i128;

    for triangle in triangles {
        let area2 = determinant(triangle[0], triangle[1], triangle[2]).abs();

        total_area2 += area2;

        let sum_x = triangle[0].x as i128 + triangle[1].x as i128 + triangle[2].x as i128;

        let sum_y = triangle[0].y as i128 + triangle[1].y as i128 + triangle[2].y as i128;

        centroid_x_sum += area2 * sum_x;

        centroid_y_sum += area2 * sum_y;
    }

    if total_area2 == 0 {
        return None;
    }

    // Cada pixel:
    //
    // 10 m × 10 m = 100 m²
    //
    // area2 = 2 × área
    //
    // área_m² = area2 / 2 × 100

    let area_m2 = total_area2 * 50;

    let centroid_x = centroid_x_sum as f64 / (3.0 * total_area2 as f64);

    let centroid_y = centroid_y_sum as f64 / (3.0 * total_area2 as f64);

    Some((area_m2, centroid_x, centroid_y))
}

// ============================================================
// MAIN
// ============================================================

fn main() {
    let size = 512usize;

    let row_offset = 2200usize;
    let col_offset = 7200usize;

    let scene = process_initial_scene(
        "./data/B04.tif",
        "./data/B08.tif",
        "./data/SCL.tif",
        row_offset,
        col_offset,
        size,
        400,
        true,
    );

    // Por enquanto:
    //
    // região válida sem vegetação.
    //
    // Depois substituímos isso pela
    // comparação temporal t0 -> t1.

    let analysis_mask = generate_open_mask(&scene);

    generate_grayimage(&analysis_mask, size as u32, "./data/analysis_mask.png");

    // --------------------------------------------------------
    // FLOOD FILL
    // --------------------------------------------------------

    let (labels, component_count) = process_flood_fill(&analysis_mask, size);

    println!("\nManchas encontradas: {}", component_count);

    // --------------------------------------------------------
    // BORDA VISUAL
    // --------------------------------------------------------

    let border_mask = generate_border_mask(&labels, size);

    generate_grayimage(&border_mask, size as u32, "./data/border_mask.png");

    // --------------------------------------------------------
    // PROCESSAR CADA MANCHA
    // --------------------------------------------------------

    for label in 1..=component_count {
        let edges = process_border(&labels, size, label);

        let Some(polygon) = generate_polygon(&edges) else {
            println!("Mancha {}: contorno inválido ou com furo.", label);

            continue;
        };

        let polygon = simplify_polygon(&polygon);

        let Some(triangles) = triangulate(&polygon) else {
            println!("Mancha {}: falha na triangulação.", label);

            continue;
        };

        let Some((area_m2, centroid_x, centroid_y)) = calculate_area_centroid(&triangles) else {
            continue;
        };

        // Centroide dentro da cena inteira
        let global_col = col_offset as f64 + centroid_x;

        let global_row = row_offset as f64 + centroid_y;

        println!("\n--- Mancha {} ---", label);

        println!("Vértices: {}", polygon.len());

        println!("Triângulos: {}", triangles.len());

        println!("Área: {} m²", area_m2);

        println!("Área: {:.4} ha", area_m2 as f64 / 10_000.0);

        println!("Centroide local: ({:.3}, {:.3})", centroid_x, centroid_y);

        println!(
            "Centroide global da imagem: ({:.3}, {:.3})",
            global_col, global_row
        );
    }
}
