# CONTEXTO_PROJETO.md — transferência de contexto para Codex

**Projeto:** `deforestation-monitor`  
**Data da consolidação:** 30/10/2026  
**Estudante:** Filipe Gomes de Souza Matozo  
**Curso informado para a submissão:** Bacharelado de Engenharia de Controle e Automação  
**Instituição:** Ifes Campus Serra  
**Docente/orientador mencionado:** Fidelis Zanetti de Castro  
**Disciplina:** Introdução à Geometria Analítica

> Este arquivo registra decisões e resultados **informados durante a conversa**. Não substitui inspeção do checkout nem demonstra que os últimos códigos/ZIPs já foram testados ou incorporados.

## 1. Objetivo e escopo científico

Desenvolver uma aplicação em Rust para processar imagens Sentinel-2 da Amazônia, segmentar pixels classificados como vegetação por NDVI, extrair manchas por conectividade e polígonos, calcular suas áreas via determinantes (fórmula do cadarço) e georreferenciar vértices. Próximo objetivo: comparar a mesma região em duas datas, mapear **possíveis alterações** na cobertura vegetal e gerar evidências visuais para um miniartigo. Um professor pode avaliar o trabalho e eventualmente indicar publicação em um veículo do Ifes; **não há submissão ou aceite confirmado**.

A prioridade pedagógica é demonstrar matrizes, transformação afim, determinantes, polígonos e coordenadas, não alegar monitoramento oficial de desmatamento.

## 2. Organização e dependências existentes

- Linguagem: Rust; bibliotecas mencionadas: `tiff_reader`, `geotiff_reader`, `ndarray`, `image`.
- O programa inicial estava concentrado em `src/main.rs`.
- Foram geradas sucessivas versões de código fora do repositório, com nomes:
  - `main.rs` (base)
  - `main_area_individual.rs` (áreas por componente)
  - `main_georreferenciado.rs` (versão com biblioteca para conversão)
  - `main_manual_coordinates.rs` (conversão manual; preferida para a exigência do professor)
  - `main_temporal.rs` (comparação temporal proposta)
- Posteriormente foi entregue um ZIP chamado `Sentinel2_Web_Interativo_v2.zip`, cuja estrutura, **caso copiada para o checkout**, adiciona:
  - `src/bin/web.rs` — servidor local e upload de TIFF;
  - `src/bin/temporal.rs` — processamento de T1/T2;
  - `src/bin/web_home.html` — formulário/visualização;
  - `src/bin/dashboard.html` — painel.
- O ZIP declarava não substituir `src/main.rs`. O servidor web usa `cargo run --bin temporal` como subprocesso e produz resultados na pasta `resultado_temporal/`. Os uploads são salvos em `web_uploads/`. A arquitetura é protótipo e **não equivale a um núcleo Rust compartilhado em `src/lib.rs`**; decidir com base no estado real do repositório.
- Para iniciar o protótipo web: `cargo run --bin web`, abrir `http://127.0.0.1:3000`.
- **Estado de execução:** versões temporal e web foram fornecidas, mas **não há confirmação de `cargo check`, `cargo test` ou uso ponta a ponta na máquina do usuário**. Inspecione antes de confiar.

## 3. Pipeline geométrico anteriormente executado

1. Ler janela 512×512 das bandas B04 e B08 (10 m/pixel) e janela correspondente da SCL (20 m/pixel).
2. Classificar pixels válidos, anteriormente com SCL ∈ {4,5}, e NDVI aproximadamente `(NIR − RED)/(NIR + RED)`, calculado como inteiro escalado por 1000; limiar usado = `400` (NDVI 0,40).
3. Gerar máscaras `valid` e `vegetation`.
4. `process_flood_fill`: BFS com `VecDeque`, vizinhança de 4 conexões, rótulos inteiros.
5. `process_edge`: produzir arestas unitárias direcionadas ao redor de cada pixel da máscara.
6. `connect_all_edges`: reconstruir os contornos com `HashMap` e regras de direção, incluindo contornos internos e possíveis toques diagonais.
7. `calculate_signed_area`, `calculate_total_area`: somar produtos cruzados/determinantes com sinal; contornos internos subtraem área. Verificar área da máscara com contagem de pixels.
8. `calculate_component_areas` ou variante otimizada: área individual das componentes, com número de pixels e contornos.
9. Georreferenciar vértices dos polígonos por transformação afim e inversão de UTM para latitude/longitude implementadas manualmente.

A complexidade topológica do método deve continuar sendo testada com máscaras sintéticas; a coincidência de área num caso real não prova que todos os polígonos possíveis estarão corretos.

## 4. Metadados reais apresentados pelo usuário

Saída do programa `geotiff_reader` para **uma** banda B04:

```text
Largura: 10980
Altura: 10980
EPSG: Some(32720)
Transformação: GeoTransform {
  origin_x: 300000.0,
  pixel_width: 10.0,
  skew_x: 0.0,
  origin_y: 9900040.0,
  skew_y: 0.0,
  pixel_height: -10.0
}
Limites: [300000.0, 9790240.0, 409800.0, 9900040.0]
Origem: (300000.0, 9900040.0)
Início do recorte: (372000.0, 9878040.0)
Fim do recorte: (377120.0, 9872920.0)
```

- CRS EPSG:32720 = WGS84 / UTM zona 20 sul.
- Recorte: `row_offset=2200`, `col_offset=7200`, `size=512`.
- Pixel local `(x,y)` corresponde à coluna original `7200+x` e linha original `2200+y`.
- Matrizes/transformação: `E = 300000 + 10*c`, `N = 9900040 − 10*r` (para **esse** raster, sem rotação).
- O determinante da matriz linear `[[10,0],[0,-10]]` é −100; módulo = **100 m² por pixel** = 0,01 ha.
- A janela cobre **262.144 pixels** = 2.621,44 ha (26,2144 km²), não confundir com área de vegetação classificada.
- A transformação da janela, e NÃO necessariamente os valores hardcoded do exemplo, deve vir dos metadados do TIFF em futuras cenas.
- O professor quer que as **conversões matemáticas sejam programadas e explicadas**, não delegadas integralmente a uma biblioteca de UTM; leitura de metadados pela biblioteca é aceitável.

## 5. Resultados geométricos reportados — **primeira cena apenas**

A execução reportada no terminal foi:

```text
Componentes encontrados: 19
Arestas encontradas: 8634
Contornos encontrados: 62
Area por determinantes: 242802 pixels²
Area por contagem: 242802 pixels²
Area projetada aproximada: 2428.0200 ha
Total das 19 manchas: 2428.0200 ha
```

Áreas individuais reportadas (hectares, mesma máscara):

| Mancha | ha | Mancha | ha |
|---|---:|---|---:|
| 1 | 737,52 | 11 | 0,01 |
| 2 | 1644,67 | 12 | 0,79 |
| 3 | 37,98 | 13 | 0,01 |
| 4 | 1,79 | 14 | 0,04 |
| 5 | 0,01 | 15 | 0,01 |
| 6 | 0,02 | 16 | 0,32 |
| 7 | 0,16 | 17 | 0,08 |
| 8 | 0,02 | 18 | 0,04 |
| 9 | 0,04 | 19 | 4,47 |
| 10 | 0,04 | | |

Interpretação apropriada: **2.428,02 ha de pixels classificados como vegetação no recorte e sob a regra implementada**, não 2.428,02 ha de desmatamento, nem estimativa de toda a cena Sentinel-2. O resto do recorte inclui pixels inválidos; não chamar todos de não vegetação.

## 6. Comparação temporal: o que foi solicitado

A pessoa usuária deseja comparar duas cenas Sentinel-2 da mesma região para localizar manchas que passaram de vegetação em T1 para não vegetação em T2, e vice-versa, com máscaras válidas em ambas as datas. O plano era:

```text
Cena T1 (B04+B08+SCL) → NDVI/máscara T1 ┐
                                      ├→ comparar pixels válidos nas duas datas
Cena T2 (B04+B08+SCL) → NDVI/máscara T2 ┘
                                         ↓
                               possível perda / possível ganho
                                         ↓
                           BFS → contornos → área → UTM
```

O arquivo `main_temporal.rs` e a versão web v2 incluem implementação proposta dessa ideia, mas **não há resultados temporais reais verificados**. O teste inicial recomendado é T1=T2 → perdas e ganhos zero.

### Cuidados antes de publicar resultados

- Registrar datas de aquisição, IDs dos produtos e proveniência de B04/B08/SCL em T1 e T2. Essas informações **ainda não foram fornecidas de forma verificável**.
- Conferir coincidência espacial de ambas as cenas, não apenas o EPSG; comparar origem, resolução, grade e eventual deslocamento.
- Verificar `BOA_ADD_OFFSET`/escala radiométrica nos metadados do produto e o processamento dos DN. Na versão temporal há parâmetros para offsets, mas **sua correção para os dados reais não foi verificada**.
- Verificar se o método atual de amostragem SCL 20→10 m (`i/2`, `j/2`) é correto considerando o grid e os offsets, inclusive para linhas/colunas ímpares. No protótipo, assume alinhamento de 2:1 e recorte de tamanho par.
- A política SCL `{4,5}` é restritiva, porque exclui outras classes e pode reduzir a região comparável; justificar metodologicamente e avaliar consequências.
- Diferença de NDVI pode resultar de sazonalidade, atmosfera, manejo agrícola, nuvens residuais, sombras e classificação; só chamar de desmatamento após validação externa.
- A imagem toda mede 10980×10980 pixels; o sistema atual **não percorre os 484 blocos** 512×512 da cena. Generalizar para toda a imagem exigirá tratamento de bordas e componentes cruzando blocos.

## 7. Interface web solicitada

O usuário rejeitou uma web que apenas abre PNGs previamente gerados; quer **escolher os TIFFs pelo navegador** e visualizar todas as fases sem informar caminhos nem abrir arquivos manualmente.

Requisitos de experiência:
- Um comando para iniciar: `cargo run --bin web`;
- upload local de B04, B08, SCL para T1 e T2;
- opção de duplicar T1 em T2 para verificar identidade;
- configuração do recorte, NDVI e offsets;
- botão para processar;
- abas de bandas, NDVI, máscara válida, máscara vegetal, perdas, ganhos, arestas, sobreposição e resultados;
- download de PNG/CSV para inserção no artigo;
- sem servidor na internet e sem banco de dados.

Foi gerado o pacote `Sentinel2_Web_Interativo_v2.zip`, usando `web.rs` como servidor local e `temporal.rs` como executável de cálculos. Não pressupor que o usuário o instalou, executou ou aprovou. A implementação atual é protótipo para validação, não produto final.

## 8. Miniartigo e submissão acadêmica

### Dois PDFs de referência discutidos

1. `projetos_latex_overleaf (2)-2.pdf`: exemplo técnico de geometria, máscaras sintéticas, fórmulas de área, calibração, figuras, comparação de datas simuladas, análise de sensibilidade, resultados, discussão de limitações e referências. Importante: **os valores 3, 7, 10 ha nesse PDF são sintéticos e não são resultados do projeto atual**.
2. `Trabalho I - Probabilidade e Estatística (1).pdf`: exemplo de organização de texto acadêmico com fundamentação, experimentos, tabelas, discussão, bibliografia, apêndices e declaração de uso de IA. Seu conteúdo tem tema diferente e não deve ser transplantado para a análise de satélite.

### Estrutura combinada proposta

1. Título, autor, instituição, resumo e palavras-chave.
2. Introdução (problema, contexto, objetivo, contribuição matemática).
3. Fundamentação (Sentinel-2, NDVI, SCL, determinantes, contornos, projeções).
4. Materiais e métodos (cenas, datas, código Rust, processamento, validações).
5. Resultados (ensaios sintéticos; primeira cena; comparação temporal; figuras).
6. Discussão (sensibilidade NDVI, confiança na classificação, área e limitações).
7. Conclusão.
8. Referências verificadas, eventuais apêndices, reprodução e declaração de IA conforme normas do professor/revista.

Título de trabalho sugerido: **APLICAÇÃO DE GEOMETRIA ANALÍTICA NA ANÁLISE DE ALTERAÇÕES DA COBERTURA VEGETAL AMAZÔNICA A PARTIR DE IMAGENS SENTINEL-2**. Provisório.

Experimentos propostos:
- E1: validação geométrica com máscaras sintéticas conhecidas;
- E2: classificação/área da primeira cena Sentinel-2;
- E3: comparação T1/T2 com locais e condições comparáveis;
- E4: sensibilidade ao limiar NDVI (por exemplo 0,35/0,40/0,45, como valores experimentais a justificar).

Figuras: fluxo do pipeline; imagens originais (para RGB real exigir B02+B03+B04 — B04+B08 não bastam); NDVI T1/T2; máscaras; possíveis mudanças; contornos; mapa com escala/coordenadas; gráfico de sensibilidade. Figuras precisam ser geradas por dados reais processados, não simuladas para aparentarem resultados.

### Resumo anterior para Jornada do Ifes

Uma versão simples já foi redigida em outubro de 2026, com foco na geometria da primeira cena e menção à comparação temporal como continuidade. Metadados informados:
- Autor: Filipe Gomes de Souza Matozo;
- Orientador: Fidelis Zanetti de Castro;
- Curso: Bacharelado de Engenharia de Controle e Automação;
- Instituição: Ifes Campus Serra;
- Área do conhecimento mantida conforme solicitação: Ciências Exatas e da Terra / Probabilidade e Estatística Aplicada.

Esse resumo **não substitui o miniartigo**. Se a comparação temporal for validada, atualizar resultados e discussão com números reais.

## 9. Próxima sessão de trabalho: checklist recomendado

1. Fazer inventário do checkout: quais dos arquivos acima existem? qual `Cargo.toml` e qual entrada? Quais dados TIFF estão disponíveis?
2. Rodar `cargo check --all-targets` e `cargo test`; corrigir erros sem mudar geometria desnecessariamente.
3. Validar o servidor local e o processo de upload da web; reportar testes efetivamente executados.
4. Executar T1=T2 e confirmar perda/ganho zero; guardar saída de terminal e imagens.
5. Recuperar metadados completos da primeira cena; selecionar/obter T2 realmente comparável.
6. Conferir radiometria (offsets/escala) e georreferenciamento das três bandas por data.
7. Gerar primeiras figuras e tabelas reprodutíveis.
8. Reexecutar E1–E4, discutir resultados sem afirmar detecção definitiva de desmatamento.
9. Só então redigir a versão 1 do miniartigo e submeter à revisão do professor.

## 10. Prompt de continuidade para Codex

> Leia integralmente `AGENTS.md` e `CONTEXTO_PROJETO.md`. Faça primeiro um inventário dos arquivos e do estado real do projeto. Compare os artefatos temporais/web com o código existente, execute `cargo check --all-targets` e `cargo test` se houver ambiente e dependências disponíveis. Não suponha que a interface web já funciona. Priorize um teste T1=T2 e depois a comparação temporal real; mantenha a Geometria Analítica calculada em Rust. Relate alterações em arquivos, resultados de testes e limitações. Não reescreva o projeto nem faça afirmações científicas não verificadas.
