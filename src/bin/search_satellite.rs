
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();

    let url = "https://earth-search.aws.element84.com/v1/search";

    let params = json!({
        "collections": ["sentinel-2-l2a"],
        "datetime": "2025-08-01T00:00:00Z/2025-09-30T23:59:59Z",
        "bbox": [-55.65, -7.79, -55.54, -7.68],
        "query": {
            "grid:code": {
                "eq": "MGRS-21MXM"
            }
        },
        "limit": 100
    });

    let response = client
        .post(url)
        .json(&params)
        .send()
        .await?
        .error_for_status()?;

    let data: Value = response.json().await?;

    if let Some(items) = data["features"].as_array() {
        let mut results: Vec<(String, String, f64)> = Vec::new();

        for item in items {
            let id = item["id"].as_str().unwrap_or("");

            if !id.contains("_21MXM_") {
                continue;
            }

            let date = item["properties"]["datetime"]
                .as_str()
                .unwrap_or("Desconhecida");

            let cloud = item["properties"]["eo:cloud_cover"]
                .as_f64()
                .unwrap_or(100.0);

            results.push((
                id.to_string(),
                date.to_string(),
                cloud,
            ));
        }

        results.sort_by(|a, b| a.2.total_cmp(&b.2));

        println!("CENAS SENTINEL-2 — TILE 21MXM");
        println!("Periodo: agosto-setembro de 2025\n");

        for (id, date, cloud) in &results {
            println!("ID: {id}");
            println!("Data: {date}");
            println!("Nuvens: {cloud:.2}%");
            println!(
                "Link: https://earth-search.aws.element84.com/v1/collections/sentinel-2-l2a/items/{id}"
            );
            println!("-----------------------------------");
        }

        println!("Total nesta pagina: {}", results.len());
    } else {
        println!("Nenhuma cena retornada ou resposta inesperada.");
        println!("{}", serde_json::to_string_pretty(&data)?);
    }

    Ok(())
}
