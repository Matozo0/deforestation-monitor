# AGENTS.md — deforestation-monitor

## Missão
Manter e evoluir um estudo acadêmico de Geometria Analítica aplicado a imagens Sentinel-2 de uma região amazônica. O programa em **Rust** segmenta vegetação com NDVI e SCL, identifica componentes conexas, extrai polígonos e calcula áreas por **determinantes**. A próxima etapa é a comparação temporal entre duas cenas; uma web **local** serve para inspeção e exportação de figuras. Há intenção de escrever um miniartigo para avaliação docente e possível adaptação editorial.

## Antes de editar
1. Leia `CONTEXTO_PROJETO.md` e examine **os arquivos reais deste checkout** (`Cargo.toml`, `src/main.rs`, `src/bin`, testes e dados de exemplo disponíveis).
2. Diferencie **implementado**, **testado** e **planejado**; a documentação relata artefatos gerados numa conversa, não comprova que todos estão copiados para este repositório ou compilam.
3. Execute `cargo test` e `cargo check --all-targets` quando possíveis; relate erros reais sem afirmar que uma execução ocorreu se não ocorreu.
4. Faça mudanças pequenas e justificadas. Preserve o comportamento verificado e os testes existentes. Não substitua o projeto por uma reescrita ampla.

## Requisitos funcionais e matemáticos
- Cálculos científicos e geométricos em **Rust**. HTML/CSS/JS apenas para entrada, controle, visualização e downloads; não recalcular NDVI ou área no front-end.
- Manter leitura das bandas B04 (vermelho), B08 (infravermelho próximo) e SCL (classificação de cena); saída de máscaras válidas e vegetação, Flood Fill com **4-vizinhos**, extração de arestas direcionadas, contornos incluindo buracos, fórmula do cadarço com áreas orientadas, área por componente e checagem contra contagem de pixels.
- **Conversões de coordenadas precisam ser calculadas explicitamente em Rust** para demonstração ao professor. Bibliotecas podem ler *metadados* do GeoTIFF, mas não devem substituir o núcleo matemático de pixel→UTM e UTM→latitude/longitude. Validar aproximações contra pontos de referência.
- Para comparação temporal, comparar apenas pixels **válidos nas duas cenas**; verificar compatibilidade de CRS/EPSG, origem, resolução, dimensões, grade, offsets de recorte, produtos e época de aquisição. Não assumir alinhamento só porque imagens têm a mesma dimensão.
- Considerar `BOA_ADD_OFFSET` e `QUANTIFICATION_VALUE` da versão real dos produtos Sentinel-2. Não assumir offset 0 sem verificação. Examinar se SCL 20 m pode ser indexada corretamente com B04/B08 10 m; checar origem, grade e tratamento dos pixels na borda.
- **Mudança de classe NDVI ≠ desmatamento confirmado.** Rotular como "possível perda de cobertura vegetal" ou "alteração na classificação vegetal" até haver validação independente.
- Não confundir área da janela (512×512×100 m²) com a área de vegetação nela. A soma de área por determinantes igual à contagem valida a geometria da máscara, não a classificação espectral.
- A análise atual é de **um recorte 512×512**, não da cena toda (10980×10980). Não afirmar processamento integral; mosaico e fusão de componentes nas fronteiras de blocos ficam para etapa posterior.

## Interface web — escopo mínimo
- Inicialização ideal: `cargo run --bin web`, servido apenas em `127.0.0.1:3000`.
- Seleção no navegador dos seis TIFFs (T1/T2: B04, B08, SCL), com possibilidade de usar os arquivos de T1 como T2 para teste de identidade. Arquivos podem ser grandes: upload em streaming para **disco local**, sem armazenamento integral em RAM.
- Exibir B04, B08, NDVI, máscaras de validade/vegetação, mapas de possível perda e ganho, arestas/contornos, métricas e PNG/CSV para exportar.
- Interface deve facilitar depuração e geração de figuras, **não integrar necessariamente o miniartigo**.
- Testar a interface realmente, inclusive falhas de upload, arquivos incorretos e parâmetros inválidos. Evitar executar subprocessos arbitrários com entrada do usuário; jamais expor servidor na rede pública sem uma revisão de segurança.
- Não reimplementar o pipeline na web. Extraia biblioteca comum somente se isso reduzir duplicação e preservar verificabilidade. É aceitável manter binários separados no início.

## Experimentos e verificações obrigatórios
- `cargo test` e `cargo check --all-targets`; para a web, `cargo run --bin web` e teste ponta a ponta.
- Prova de identidade T1=T2: possível perda = 0 e ganho = 0 (com mesmas máscaras, alinhamento e parâmetros).
- Testes sintéticos: componente isolada, L, região com furo, toques diagonais, soma de áreas orientadas e componentes com múltiplos contornos.
- Conferência das coordenadas do recorte pelos metadados do arquivo, preservando EPSG e valores reais.
- Registrar: versões de dependências, datas e IDs das cenas, resolução/CRS, regra SCL, NDVI/offsets/limiares, recorte, contagens, área e limitações.

## Forma de trabalhar com a pessoa usuária
- Conversar em português brasileiro, de forma didática e objetiva.
- Priorizar explicação do que mudou, por que e como testar. Entregar código utilizável quando solicitado.
- Evitar complexidade desnecessária, refatorações abrangentes, serviços pagos, hospedagem e dependências sem necessidade.
- Antes de cada alteração importante, apontar os testes de aceitação e os arquivos impactados.
- Nunca inventar resultados, datas, imagens, validações, referências bibliográficas ou afirmações sobre publicação.

## Primeira tarefa sugerida
Auditar o checkout atual, identificar o que efetivamente foi incorporado da versão web e temporal, executar `cargo check --all-targets`, corrigir problemas de compilação e estabelecer um teste ponta a ponta com T1=T2 antes de tratar duas datas reais. Leia `CONTEXTO_PROJETO.md` para números de referência e decisões.
