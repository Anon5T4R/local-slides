# ROADMAP — Local Slides

Fork do deckcraft (deckcraft v0.15.1), rebrandado para a suíte Local. Rust puro + egui, desktop.
O formato nativo é o `.deckcraft` do upstream (zip + JSON, mantido byte a byte, com o MIME
`application/vnd.storyteller.deckcraft+zip`); PPTX entra e sai. Como é construído:
`docs/architecture.md`; o que vem do upstream: `docs/UPSTREAM-SYNC.md`.

**Estágio: alpha.** Abre, edita, apresenta e salva decks de uso real; o que abaixo falta é o caminho
até "pronto para trabalho de verdade".

## O que falta (por área, sem ordem de dificuldade)

- **Fidelidade do PPTX.** O corpus de round-trip é só de decks que nós autoramos: falta conferir
  contra renderizações do PowerPoint e decks de terceiros; opções de mídia ainda se perdem num
  round-trip.
- **Estabilidade.** O fechamento de um deck editado precisa perguntar antes (hoje só o encerramento
  pergunta), e o app não abre em algumas GPUs Windows (enumeração de Vulkan).
- **Imagens.** SVG não renderiza (EMF/WMF sim); colar imagem no Windows falha em alguns casos.
- **Animações.** O painel cobre parte dos efeitos do PowerPoint (timeline e Play From já existem).
- **Gráficos.** Falta editar os dados na grade e os elementos do gráfico.
- **Formatação.** O painel de Formatar Forma é mais enxuto que o do PowerPoint; fontes só nas faces
  Regular, Negrito e Itálico.
- **Impressão.** Não imprime.
- **Vídeo do deck.** Exporta GIF animado, não MP4/MOV.
- **Apresentação.** A visão do apresentador não se posiciona na segunda tela.
- **Desempenho.** Os slides rasterizam na CPU, sem baseline de performance.
- **Localização.** Inglês, português (Brasil) e espanhol; os catálogos ainda têm strings sem
  tradução (`cargo xtask i18n-report`).

## Formatos

| Formato | Ler | Gravar |
| --- | --- | --- |
| `.pptx` | sim | sim |
| `.potx` / `.ppsx` | sim (como apresentação) | sim, com o content type próprio |
| `.deckcraft` (nativo; também `.slidecraft`) | sim | sim, sem perdas |
| `.thmx` (tema do Office) | só cores, fontes e efeitos (Design › Procurar Temas…) | não |
| PPTX com macro (`.pptm`/`.potm`/`.ppsm`), `.ppt` 97–2003, `.odp` | não | não |
| Outline `.txt`/`.rtf`/`.md` | sim (títulos viram slides, itens viram marcadores) | `.txt`/`.rtf` |
| PDF | — | sim (slides, anotações, folhetos); a arte do slide entra rasterizada |
| PNG/JPEG por slide | — | sim (com fundo transparente opcional) |
| GIF animado do deck | — | sim (transições e animações não entram) |
| TIFF/BMP/GIF por slide, MP4/MOV | — | não |

## Referências

`docs/architecture.md` · `docs/control-protocol.md` · `docs/mcp.md` · `i18n/README.md` ·
`docs/UPSTREAM-SYNC.md`
