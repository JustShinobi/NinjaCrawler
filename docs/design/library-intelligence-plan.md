# Plano executável: Library Intelligence

**Status:** F0–F6 implementadas (não commitadas) — falta validação manual
**Criado:** 2026-07-31
**Base:** `develop` @ `38c9edd`
**Branch atual:** `feat/library-intelligence` (contém F0+F2; renomeada de `feat/media-index`)
**Escopo:** itens 4, 7, 8, 11, 12 e 13 da sessão de sugestões

---

## Log de execução

### 2026-07-31 — F0 (`media_index`) implementada

**Backend**
- `migrations/0051_media_index.sql`: tabelas `media_index` e `media_index_runs` + 8 índices.
  Registrada em `database.rs` com reparo idempotente (`ensure_media_index_schema`) dentro de
  `reconcile_colliding_development_migrations`, seguindo o padrão já usado para colisões de
  migration entre branches.
- `workspace_repository/media_index.rs` (novo): upsert com id estável, herança de fingerprints
  do `media_dedupe_catalog`, reconciliação disco↔índice, contadores e persistência de runs.
- Hook de ingest em `upsert_provider_sync_media_ledger_entries`
  ([media.rs:1099](../../src-tauri/src/infrastructure/workspace_repository/media.rs:1099)) —
  best-effort: falha de índice registra no runtime log e **não** derruba o sync.
- `infrastructure/media_index_runtime.rs` (novo): fila em thread com progresso por perfil,
  cancelamento, evento `media-index://status-changed`, recuperação de runs órfãos no boot
  (ligada em `desktop_runtime::start_runtime_services`).
- Comandos `media_index_status` / `start_media_index_scan` / `cancel_media_index_scan`,
  registrados em `lib.rs`.

**Frontend**
- `MediaIndexCounts` / `MediaIndexRun` / `MediaIndexStatus` em `domain/models.ts`.
- Parsers manuais + subscrição do evento em `bridge/desktop.ts` (camelCase e snake_case).
- Painel "Media index" no topo da aba **Storage & cleanup** do Workspace Health: contadores,
  progresso por perfil, iniciar/parar.

**Decisões tomadas durante a implementação**
- O `media_dedupe_catalog` foi mantido intacto; a herança de hashes junta por `normalized_path`,
  então `normalize_absolute_media_path` replica exatamente a forma usada pelo dedupe (barra
  invertida, minúsculas no Windows). Divergir aí faria a herança casar zero linhas em silêncio.
- Herança só ocorre quando `size_bytes` **e** `modified_at_ms` batem com o catálogo — hash de
  outra revisão do arquivo é pior que hash nenhum.
- Reindexar arquivo alterado preserva o `id` mas zera hashes, `variant_group_id` e
  `is_canonical`; o id é o que coleções (F4) e grupos de variantes (F5) vão referenciar.
- Trilha de slideshow (`*_audio.*`) e diretórios que começam com `.` ficam fora do índice.
- Progresso é contado em perfis, não em arquivos: publicar por arquivo custaria mais que o
  valor que entrega.

**Validação**
- `cargo test --lib`: 377 passed, 0 failed (13 testes novos de `media_index`).
- `npm test`: 762 passed, 104 arquivos.
- `npm run lint`: 0 erros (1 warning pré-existente de `useVirtualizer`).
- `tsc -b`: limpo.
- **Pendente: validação manual em biblioteca real** — indexar acervo grande, medir tempo,
  conferir herança de hashes e o comportamento de cancelamento.

### 2026-07-31 — F2 (removido da origem) implementada

**Correção retroativa na F0.** A migration 0051 usava `upstream_state` para "sumiu do disco".
São dois eixos independentes — um arquivo pode sumir do disco e continuar online, ou estar
arquivado aqui e removido do provider. Como a F0 ainda não estava commitada, a coluna foi
renomeada para `local_state` e `upstream_state` ficou com o significado correto, em vez de
empilhar uma migration corretiva.

**Backend**
- `migrations/0052_upstream_presence.sql` + reparo idempotente `ensure_upstream_presence_schema`
  (as colunas são `ALTER TABLE`, então precisam de `add_column_if_missing`). Cobre os **dois**
  ledgers de post: o provider-neutro e o `instagram_sync_post_ledger`, que tem esquema próprio
  e é criado em runtime.
- `workspace_repository/upstream_presence.rs` (novo): qualificação da varredura, avaliação com
  contagem de confirmações, projeção no `media_index`, trilha de auditoria em
  `source_full_scan_runs`.
- Ligado nos **5 providers**, cada um com o seu sinal de enumeração completa:
  | Provider | Sinal de "enumerou a seção inteira" |
  | --- | --- |
  | Instagram | `request.full_scan` (desliga o early-stop da descoberta) |
  | X/Twitter | `incremental_cutoff_timestamp` ausente **e** sem resume cursor |
  | TikTok | sem early-stop no provider; basta não ser download de vídeo único |
  | YouTube | sem early-stop; yt-dlp lista o canal a cada run |
  | VSCO | sem early-stop |

**Frontend**
- `upstreamMissing` em `MediaGalleryPost` (Rust + TS + parser do bridge).
- Badge âmbar "Archived only" no card e filtro "Archived here only" no popover de filtros
  avançados do ProfileView, incluído nos presets salvos.

**Decisões tomadas durante a implementação**
- **Seções efêmeras ficam fora.** Stories expiram sozinhos e likes do TikTok somem quando o
  dono descurte — marcar isso como "removido pelo autor" encheria o acervo de ruído. Só
  timeline/feed/reels/videos/shorts/photos/reposts/gallery são julgados.
- **Duas confirmações** antes de marcar. Uma listagem curta pode vir de um soluço do provider
  que daqui parece limpo.
- **Mídia deletada localmente nunca conta.** O sync deliberadamente não a rebaixa, então ela
  está sempre ausente da listagem — é decisão do operador, não remoção do autor.
- **Posts incompletos truncam a avaliação** (Twitter `incomplete_post_count`): eles são
  descartados de `observed_posts`, e tratá-los como ausentes marcaria posts que só falharam
  parcialmente naquele run.
- Badge com `aria-hidden`, como os selos irmãos — a thumb é um `<button>` e texto legível
  dentro dela entra no nome acessível.

**Validação**
- `cargo test --lib`: 385 passed, 0 failed (8 testes novos de `upstream_presence`).
- `npm test`: 764 passed. `npm run lint`: 0 erros. `tsc -b`: limpo.
- **Pendente: validação manual real** — rodar full scan em perfil com post sabidamente
  apagado e confirmar que o badge aparece só na segunda varredura.

### 2026-07-31 — F1 (identidade e renomeação) implementada

**O que já existia** (e mudou o escopo previsto): `detect_duplicate_user_id_on_first_sync` já
rodava nos providers, os cinco connectors já resolviam o user id, e IG/Twitter/TikTok/YouTube/
VSCO já tinham funções `persist_*_user_id_hint`. Faltava o essencial: o id ficava dentro do
`sync_options_json`, e `source_user_id_hint_from_json` só sabia ler Instagram e X — ou seja, os
hints gravados para TikTok, YouTube e VSCO **nunca eram lidos** e a proteção contra perfil
duplicado era inerte nesses três providers.

**Backend**
- `migrations/0053_source_identity.sql` + `ensure_source_identity_schema`: coluna
  `provider_user_id` (indexada por `(provider, provider_user_id)`), `identity_id`, tabelas
  `identities` e `source_handle_history`.
- `workspace_repository/source_identity.rs` (novo): `record_source_identity` classifica cada
  sync em `Unchanged` / `Adopted` / `Renamed` / `HandleRecycled`, mantém o histórico de handles
  e `apply_source_identity_verdict` age sobre o veredito.
- `find_source_with_same_user_id` passou a usar a coluna indexada, com fallback ao JSON legado
  para perfis que ainda não sincronizaram desde a migration. A varredura O(n) com parse de JSON
  por linha sai do caminho quente.
- Ligado nos 5 providers, ao lado das funções `persist_*_user_id_hint` existentes.
- Comandos: `list_identities`, `create_identity`, `delete_identity`,
  `link_source_to_identity`, `suggest_identity_links`, `load_source_handle_history`.

**Frontend**
- `providerUserId`/`identityId` em `SourceProfile`, tipos `Identity`,
  `IdentityLinkSuggestion` e `SourceHandleHistoryEntry` + parsers no bridge.
- Linha "formerly @antigo" no header do ProfileView.
- Badge "Handle taken over" em `syncProblemBadges` — o problema já flui para o Workspace
  Health e para a lista de perfis pelo caminho de `sync_problem_code` que já existia.

**Decisões tomadas durante a implementação**
- **Handle reciclado pausa o perfil** (`ready_for_download = 0`) e **nunca** sobrescreve o
  `provider_user_id` guardado: o perfil no workspace continua se referindo à pessoa cujo
  acervo está no disco. Retomar é ação deliberada do operador — se isso acontecesse em
  silêncio, seria corrupção irreversível do arquivo.
- **Vínculo de identidade é sempre manual.** `suggest_identity_links` só compara handles
  idênticos entre providers e nunca aplica sozinho: um falso positivo aqui alimentaria o
  dedupe cross-provider da F5 com duas pessoas diferentes.
- O hint no `sync_options_json` foi mantido como fallback de leitura em vez de migrado num
  backfill — a coluna se preenche sozinha no próximo sync de cada perfil.

**Validação**
- `cargo test --lib`: 390 passed, 0 failed (5 testes novos de `source_identity`).
- `npm test`: 765 passed. `npm run lint`: 0 erros. `tsc -b`: limpo.
- **Pendente: validação manual real** — em especial confirmar que um perfil renomeado de fato
  cai em `Renamed` (e não em `HandleRecycled`) com os ids reais de cada provider.

### 2026-07-31 — F3 (timeline agregada) implementada

**Backend**
- `workspace_repository/media_timeline.rs` (novo): paginação keyset por
  `(captured_at DESC, id DESC)` sobre o `media_index`, com `GROUP BY` por post para que um
  carrossel seja um card só — o mesmo que o grid do perfil já faz.
- Filtros: providers, perfis, identidades, seções, tipo de mídia, janela de datas,
  `upstreamMissingOnly` e `unseenOnly`. Todos ligados por binding; só a quantidade de
  placeholders varia com o filtro.
- Contador de novidades ancorado em `library.timeline.lastSeenAt` (settings), com
  `mark_timeline_seen`.
- Comandos `load_media_timeline`, `mark_timeline_seen` e `open_library_window`.

**Frontend**
- Janela nova: `library.html` + `src/library.tsx` + `features/library/LibraryWindowPage.tsx`,
  registrada em `vite.config.ts`, `desktop_runtime.rs` (label + builder) e no toolbar do App.
- Timeline agrupada por dia, scroll infinito via `IntersectionObserver`, reuso do `MediaCard`
  (com o badge "Archived only" da F2) e dos thumbnails gerados sob demanda.

**Decisões tomadas durante a implementação**
- **Keyset, não offset.** Mídia nova chega no topo enquanto o operador rola; com offset a
  paginação repetiria e puliria itens. O cursor trata `captured_at` nulo como o menor valor
  possível, senão posts sem data sumiriam da segunda página em diante.
- **`GROUP BY COALESCE(provider_post_key, id)`**: sem post key (biblioteca importada) cada
  arquivo vira seu próprio card. O pior caso é um post aparecer como vários cards — nunca dois
  posts diferentes fundidos num só.
- **Caminho absoluto reconstruído** a partir do profile root + `relative_path`, resolvido uma
  vez por perfil e não por linha. O `normalized_path` do índice é lowercase e não serve para
  exibir.
- Arquivos com `local_state = 'missing_on_disk'` ficam fora da timeline: continuam no índice
  para o relatório de saúde, mas não há o que renderizar.
- Sem marca de "visto", o contador de novidades fica em zero em vez de declarar a biblioteca
  inteira como nova.

**Validação**
- `cargo test --lib`: 395 passed, 0 failed (5 testes novos de timeline, incluindo a varredura
  completa por keyset sem repetição).
- `npm test`: 770 passed (5 testes novos da janela). `npm run lint`: 0 erros. `tsc -b`: limpo.
- **Pendente: validação manual real** — abrir a Library com biblioteca grande e medir o tempo
  da primeira página e do scroll infinito.

### 2026-07-31 — F4 (coleções), F5 (dedupe inteligente) e F6 (dashboard) implementadas

**F4 — coleções**
- `migrations/0054_collections.sql` + `workspace_repository/collections.rs`.
- `scope` (global/source/identity) e `kind` (manual/smart) como dimensões ortogonais na mesma
  tabela: promover uma coleção de perfil para global é `UPDATE` de duas colunas e os itens
  ficam — foi o que o desenho prometeu.
- **Um motor só, como planejado:** `collectionId` virou um filtro da timeline. Coleção manual
  filtra por pertencimento, smart expande a `rule_json` (que é o próprio `MediaTimelineFilter`
  serializado). Ambas paginam pelo mesmo keyset.
- Coleção com escopo nunca vaza mídia de fora do dono, mesmo que a regra salva diga outra coisa.
- UI: aba Coleções, "Salvar filtro como coleção", seleção múltipla na timeline → adicionar.

**F5 — dedupe inteligente**
- `migrations/0055_media_variants.sql` + `workspace_repository/media_variants.rs`.
- Assinatura de vídeo: 5 frames amostrados (10/30/50/70/90%) via ffmpeg, dHash por frame,
  match por maioria (60%). É o que torna "story e feed são o mesmo vídeo" detectável — os dois
  arquivos diferem byte a byte.
- Backlog de fingerprint no `media_index_runtime` (sha256 + hashes perceptuais + assinatura),
  seguido da detecção de variantes na mesma execução.
- **O hash perceptual é literalmente a mesma função do dedupe** (`image_hashes` virou
  `pub(crate)`): duas implementações dariam hashes incompatíveis, e o índice herda hashes desse
  mesmo catálogo.
- Regras: dois posts do mesmo perfil na mesma seção nunca são agrupados (seriam dois posts);
  cross-provider exige identidade confirmada (F1) e janela de 7 dias; mídia sem fingerprint não
  entra.
- `link_only` é o default e não move nem apaga nada — só marca o não-canônico. A cópia canônica
  é a de maior resolução, então o feed limpo vence o story cortado.

**F6 — dashboard**
- `workspace_repository/library_dashboard.rs`: totais, quebra por provider, top 20 perfis por
  disco, crescimento mensal, grupos de variantes e bytes recuperáveis.
- **Perfis parados separam `sync_failing` de `not_posting`** — a distinção que o Workspace
  Health não faz e que decide se há algo a fazer.
- UI: aba Overview (a que abre por padrão) e aba Variants com revisão e "Not duplicates".

**Validação (as seis fases juntas)**
- `cargo test --lib`: 413 passed, 0 failed.
- `npm test`: 775 passed, 105 arquivos. `npm run lint`: 0 erros. `tsc -b`: limpo.

### 2026-07-31 — Redesenho da janela Library (após 1º uso real)

O primeiro uso real expôs dois bugs e um erro de enquadramento.

**Bugs**
- A janela não arrastava, minimizava nem fechava: `capabilities/default.json` lista quais
  janelas podem usar as APIs de janela do Tauri, e `library` não estava lá. Comandos próprios
  do app não passam por capability, por isso os dados carregavam normalmente e só o chrome
  falhava.
- A barra de filtros aparecia em todas as seções: o atributo `hidden` era anulado pelo
  `display: flex` da classe. Trocado por renderização condicional.

**Erro de enquadramento** — a janela abria num relatório de números quando o uso diário é olhar
mídia, e a lista de perfis com sync falhando (que o Workspace Health já faz) ocupava a tela
inteira. Além disso, com nada indexado, quatro zeros não explicavam nada nem ofereciam saída.

**Redesenho**
- Abas → **sidebar de destinos**: New (badge de novidades), Everything, Only here, Duplicates,
  Library summary, e as coleções listadas logo abaixo como destinos de primeira classe.
- Abre em **New**: o único lugar que responde "vale abrir isso hoje?".
- Estado de primeiro uso explica que nada foi indexado e oferece **Index library** ali mesmo,
  com progresso por perfil enquanto roda.
- Perfis com sync falhando viraram **uma linha com link** para o Workspace Health, em vez de
  uma lista que duplicava aquela janela.
- Contadores da sidebar vêm do dashboard, buscado uma vez na abertura e reusado pela seção
  Summary — um fetch, dois usos.

O usuário aprovou a sidebar com a ressalva de que uma troca de layout pode virar opção depois.

**Validação:** `cargo test --lib` 413 · `npm test` 776 · lint 0 erros · `tsc -b` limpo.

### 2026-07-31 — Indexação em segundo plano + Profile View com sidebar

**Bug: indexação "travada em 100%".** A varredura das pastas é só a primeira fase; hashear
51.983 arquivos (sha256 + ffmpeg nos vídeos) é a longa. A UI mostrava só o contador de perfis,
então a segunda fase parecia parada — e pior, o painel de progresso **bloqueava a tela inteira**
enquanto a mídia já estava indexada e navegável. Agora o painel cheio só aparece com a
biblioteca vazia; a partir daí o grid assume e o hashing vira uma faixa que informa quantos
arquivos faltam. O grid também recarrega quando a fase muda para `fingerprint`, não só no fim.

**Profile View reimaginado.** A barra tinha 18 controles numa linha porque misturava duas
perguntas: *o que estou vendo* (seções) e *como estou vendo* (modo, tipo, ordem, densidade).
As seções foram para uma sidebar — o mesmo vocabulário da janela Library — e a barra ficou só
com a segunda pergunta.

Isso abriu espaço para o que faltava:
- **Coleções com escopo de perfil** na sidebar: criar (com a seleção atual), abrir, adicionar
  seleção, promover para a biblioteca e excluir.
- **"Só aqui"** e **"Duplicatas"** aplicados ao perfil — os conceitos da F2 e da F5 no lugar
  onde o operador realmente decide algo sobre eles.

**Backend**: comando `load_collection_relative_paths` (a galeria vem do disco e trabalha em
caminhos relativos; os ids do índice nunca chegam à UI) e `has_variants` em `MediaGalleryPost`.

**Detalhe de acessibilidade que quebrou 10 testes:** os chips antigos tinham `aria-label`
explícito; sem ele, rótulo e contador em spans adjacentes viram "Feed780". Os itens da sidebar
declaram o nome, e usam `aria-current` (navegação) em vez de `aria-pressed`.

O checkbox "Archived here only" saiu dos filtros avançados — virou destino, e dois caminhos
para a mesma coisa é pior que um.

**Validação:** `cargo test --lib` 413 · `npm test` 779 · lint 0 erros · `tsc -b` limpo.

### 2026-07-31 — Hashing paralelo, estimativa e controle de recursos

Dois problemas reportados no primeiro acervo grande (721 mil itens, 604 mil por hashear).

**Sem noção de quando termina.** O progresso do backlog era só "N arquivos restantes", sem
percentual, taxa ou estimativa. O run passou a registrar `fingerprints_total`,
`fingerprints_done` e `fingerprint_started_at`; a UI deriva taxa e ETA. A estimativa só aparece
depois de ~20 arquivos — antes disso ela oscila demais para ser honesta.

**Subutilização da máquina.** O backlog era estritamente sequencial: um arquivo por vez,
sha256 seguido de ffmpeg. Num PC potente isso deixa quase tudo ocioso.

Agora o backlog roda com N workers, escolhidos pelo mesmo vocabulário que a limpeza de mídia já
usa (`quiet` / `balanced` / `fast`):

| Perfil | Workers | Quando |
| --- | --- | --- |
| `quiet` | 1 | máquina precisa ficar livre |
| `balanced` (padrão) | metade dos núcleos lógicos | uso normal |
| `fast` | núcleos − 1 | a máquina vai ficar ocupada, e isso é dito na UI |

**Nada é decidido pelo app sozinho:** o padrão é `balanced`, o seletor fica visível tanto no
primeiro uso quanto na faixa de progresso, e cada opção descreve o custo (“nearly every core —
the machine will feel busy”) em vez de contar threads. Trocar de perfil no meio reinicia só o
backlog pendente; o que já foi hasheado não é refeito. A faixa também ganhou **Pause**.

Escrita concorrente: cada worker abre a própria conexão, e o SQLite já está em WAL com
`busy_timeout`, então as escritas curtas do fim de cada arquivo serializam sem falhar.

**Ainda não feito, e vale medir antes:** cada vídeo custa 5 invocações de ffmpeg (uma por frame
amostrado). Um único comando por vídeo reduziria o overhead de processo, mas trocaria seeks
rápidos por decodificação sequencial — melhor para vídeos curtos, pior para longos. Decodificação
por GPU (`-hwaccel`) é outra possibilidade, dependente do build do ffmpeg. Nenhuma das duas foi
medida ainda; o paralelismo era o ganho garantido.

**Validação:** `cargo test --lib` 413 · `npm test` 781 · lint 0 erros · `tsc -b` limpo.

**Correção seguinte — hashing retomável.** O seletor de velocidade só existia dentro de um run
ativo, e o hashing não sobrevivia a fechar o app: `recover_interrupted_media_index_runs` marca
runs `running` como falhos no boot, então ao reabrir não havia progresso, nem retomada, nem
onde configurar. Com 604 mil arquivos pendentes isso deixava a detecção de duplicatas
inalcançável.

- Comando `resume_media_fingerprints`: retoma **só** o backlog, sem re-varrer os 1434 perfis
  antes do primeiro hash.
- A faixa na Library passou a aparecer sempre que há pendências — não só durante um run —
  mostrando "Duplicate detection is paused · N files still to hash" com **Resume** e o seletor
  de velocidade sempre alcançável.
- Trocar a velocidade durante o hashing retoma só o backlog pendente, em vez de reiniciar a
  varredura de perfis.

**Validação:** `cargo test --lib` 413 · `npm test` 782 · lint 0 erros · `tsc -b` limpo.

### Ajuste no escopo restante

A F5 previa `source_links` par a par; com `identities` + `identity_id` implementados na F1, o
escopo cross-provider da F5 já tem onde se apoiar. Falta dela apenas a UI de identidades
(listar/criar/vincular numa tela), que hoje só existe como comandos.

---

## 1. Tese

Hoje o app tem um hemisfério só: **Aquisição** (providers, filas, scheduler, import). O acervo
resultante é acessível apenas perfil a perfil, derivado do disco a cada abertura.

Este plano constrói o segundo hemisfério: **Acervo** — uma camada que sabe o que existe,
de quem é, o que é repetido e o que desapareceu da origem.

As seis features escolhidas não são independentes. Cinco delas dependem da mesma peça
ausente (um índice de mídia consultável) e duas compartilham um segundo conceito ausente
(identidade de pessoa acima do perfil). O plano é ordenado por essas dependências, não
pela lista original.

```
F0  media_index  ─────────────┬──> F3 Timeline (8)
                              ├──> F4 Coleções (7)
                              ├──> F5 Dedupe inteligente (12)
                              └──> F6 Dashboard (11)
F1  Identidade (13) ──────────┴──> F5, F6
F2  Upstream missing (4) ─────────> F6
```

---

## 2. Levantamento — o que já existe

| Peça | Onde | Aproveitamento |
| --- | --- | --- |
| Catálogo incremental com sha256 + aHash/dHash | `media_dedupe_catalog` ([0045](../../src-tauri/migrations/0045_media_dedupe_incremental_vdf.sql)) | **Semente do `media_index`** — evita re-hashear a biblioteca |
| Similaridade de vídeo (VDF gerenciado) | [`media_dedupe_vdf.rs`](../../src-tauri/src/infrastructure/media_dedupe_vdf.rs) | Reusado na F5, mas hoje é **por source** |
| Fingerprints perceptuais no sync do IG | `instagram_media_fingerprints` ([0020](../../src-tauri/migrations/0020_instagram_media_fingerprints.sql)) | Prova de que hashear no ingest é viável; generalizar |
| Ledgers provider-neutros | `provider_sync_post_ledger` / `provider_sync_media_ledger` ([0023](../../src-tauri/migrations/0023_provider_sync_ledgers.sql)) | Origem de verdade da F2 |
| Tombstones de deleção local | `provider_deleted_media` ([0027](../../src-tauri/migrations/0027_deleted_media.sql)) | Precisa **não** ser confundido com upstream missing |
| Stats de perfil | `source_profiles.profile_*` ([0042](../../src-tauri/migrations/0042_source_profile_stats.sql)) | Insumo do dashboard |
| ffmpeg/ffprobe gerenciados | `media_tool_runtime`, `media_metadata_probe` | Assinatura de vídeo na F5 |
| Movimentação segura de arquivos com fila | `media_path_migration_runtime` | Renomeação de pasta na F1 |
| Padrão de janela própria | `queue-status.html`, `workspace-health.html` + `desktop_runtime.rs` | Molde da janela Library |

### Lacunas confirmadas

- Não existe índice de mídia persistido. `load_source_media_gallery` faz `read_dir` por perfil.
- Similaridade é sempre **intra-source** (`media_dedupe_source_jobs`, índice `idx_media_dedupe_files_similar`).
- Dedupe é **pós-fato e destrutivo** (scan → grupos → reclaim). Não há caminho no ingest.
- `source_profiles` não guarda o id estável do provider — só `provider_accounts` guarda.
- Nada registra que um post sumiu da origem.

---

## 3. Duas decisões de desenho pedidas

### 3.1 Coleções (item 7): escopo como atributo, não como tabela

Uma coleção tem um **escopo** e uma **natureza**. As duas dimensões são ortogonais, e é
isso que evita duas features paralelas fazendo a mesma coisa.

| | `manual` (itens escolhidos) | `smart` (regra salva) |
| --- | --- | --- |
| `scope='source'` | "Melhores do @fulano" | "Só stories do @fulano de 2025" |
| `scope='identity'` | curadoria da pessoa em IG+TikTok juntos | "tudo em vídeo dessa pessoa" |
| `scope='global'` | "Referências de edição" | "Tudo marcado como removido da origem" |

Quatro regras que fazem isso ser usado em vez de decorativo:

1. **Promoção sem perda.** Uma coleção de perfil vira global com um clique
   (`scope='global'`, `scope_ref_id=NULL`) — os itens continuam lá. O usuário começa
   local sem precisar decidir a arquitetura antes de saber se a coleção vai crescer.
2. **Coleção smart é a mesma engine da timeline.** A `rule_json` é exatamente o filtro
   da F3 serializado. Consequência: a timeline agregada é uma coleção smart sem regra, e
   qualquer filtro que o usuário montou vira coleção com um botão "salvar como coleção".
   Um motor, duas features.
3. **Custo de adicionar tem que ser uma tecla.** No lightbox, `C` abre o seletor com a
   última coleção usada em destaque; `Shift+C` adiciona direto à última usada. Sem isso a
   feature morre no primeiro mês.
4. **Coleção é referência, nunca cópia.** Um arquivo em N coleções é um arquivo no disco.
   Coleção vira depois o alvo natural de export e de política de retenção.

O escopo `source` aparece na sidebar do ProfileView; `global` e `identity`, na janela Library.

### 3.2 Dedupe (item 12): três camadas, uma delas nova

Os dois casos relatados são estruturalmente diferentes e nenhum é resolvido hoje:

| Caso | Natureza | Por que falha hoje |
| --- | --- | --- |
| Mesmo vídeo no IG e no TikTok da mesma pessoa | cross-source, cross-provider | Comparação é escopada por source; não existe vínculo entre os dois perfis |
| Story e, em seguida, feed no mesmo perfil | intra-source, cross-section | Chaves de mídia são diferentes; encodes diferem, então sha256 não bate; a similaridade só roda em scan manual posterior |

**Declaração honesta de escopo:** o download é feito por conectores externos que gravam
direto no disco. Não há como decidir *antes* de baixar — as chaves de provider são
diferentes e não há fingerprint remoto. O que este plano entrega é detecção
**pós-download, pré-catalogação**: economia de disco e de ruído visual, **não** de banda.
Isso precisa estar claro na UI para não prometer o que não entrega.

**Camada 1 — fingerprint no ingest.** Ao registrar mídia nova no `media_index`, calcular:
imagens → sha256 + aHash/dHash (código já existe no caminho do IG); vídeos → sha256 +
assinatura de N frames amostrados via ffmpeg (10/30/50/70/90% da duração), guardada como
`video_signature`. Consultar candidatos no mesmo source e nos sources da mesma identidade.

**Camada 2 — grupos de variantes persistentes.** Match acima do limiar não deleta nada:
cria `media_variant_groups` com um membro canônico. A galeria colapsa os não-canônicos em
um card com badge "2 origens" / "story + feed". O grupo sobrevive entre scans, ao contrário
dos grupos efêmeros por `scan_id` de hoje.

**Camada 3 — política configurável, `link_only` por padrão.**

| Política | Comportamento | Para quem |
| --- | --- | --- |
| `keep_all` | comportamento atual, sem agrupamento | quem não quer mudança |
| `link_only` (**default**) | mantém os arquivos, agrupa na UI | resolve a poluição visual sem risco |
| `keep_best` | move os piores para `.duplicates/` (maior resolução/bitrate vence) | recupera disco, reversível |
| `keep_first_seen` | preserva a cópia mais antiga | arquivista: o story é o efêmero, tem valor de raridade |

`keep_best` **move**, não deleta — a limpeza definitiva continua sendo o fluxo de dedupe
existente, que já tem revisão e ações explícitas.

O escopo cross-provider exige o vínculo entre perfis, senão a comparação é O(n²) sobre a
biblioteca inteira. É a F1 que fornece esse escopo.

---

## 4. Fases

### F0 — `media_index` (fundação) · tamanho **G**

Tabela canônica de item de mídia com id opaco e estável, sobrevivendo a renomeação de
pasta e a mudança de media root.

**Migration `0051_media_index.sql`**

```sql
CREATE TABLE IF NOT EXISTS media_index (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    source_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    media_type TEXT NOT NULL,
    media_section TEXT NOT NULL DEFAULT '',
    provider_media_key TEXT,
    provider_post_key TEXT,
    captured_at INTEGER,
    downloaded_at INTEGER,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    modified_at_ms INTEGER NOT NULL DEFAULT 0,
    width INTEGER, height INTEGER, duration_ms INTEGER,
    sha256 TEXT, ahash64 TEXT, dhash64 TEXT, video_signature TEXT,
    fingerprint_status TEXT NOT NULL DEFAULT 'pending',
    variant_group_id TEXT,
    is_canonical INTEGER NOT NULL DEFAULT 1,
    upstream_state TEXT NOT NULL DEFAULT 'present',
    indexed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (source_id, relative_path),
    FOREIGN KEY (source_id) REFERENCES source_profiles(id) ON DELETE CASCADE
);

CREATE INDEX idx_media_index_timeline    ON media_index(captured_at DESC, id);
CREATE INDEX idx_media_index_source      ON media_index(source_id, captured_at DESC);
CREATE INDEX idx_media_index_provider    ON media_index(provider, captured_at DESC);
CREATE INDEX idx_media_index_sha256      ON media_index(sha256);
CREATE INDEX idx_media_index_fingerprint ON media_index(fingerprint_status, updated_at);
CREATE INDEX idx_media_index_variant     ON media_index(variant_group_id);
CREATE INDEX idx_media_index_normalized  ON media_index(normalized_path);
```

**Três vias de preenchimento** (nesta ordem de prioridade, para não pagar hashing duas vezes):

1. **Ingest** — hook em `upsert_provider_sync_media_ledger_entries`
   ([media.rs:1099](../../src-tauri/src/infrastructure/workspace_repository/media.rs:1099)),
   ponto por onde toda mídia de todo provider já passa. Grava a linha; enfileira o fingerprint.
2. **Herança do dedupe** — `INSERT ... SELECT` de `media_dedupe_catalog` por
   `normalized_path`: sha256, hashes perceptuais e dimensões já calculados chegam de graça.
3. **Job de indexação** — fila em background que varre os profile roots e concilia o
   restante (importados do SCrawler/4K Stogram, arquivos mexidos fora do app).

**Novo módulo:** `src-tauri/src/infrastructure/media_index_runtime.rs` (fila + progresso +
cancelamento, no molde de `media_dedupe_runtime.rs`), e
`workspace_repository/media_index.rs` (queries).

**Comandos:** `load_media_index_status`, `start_media_index_scan`, `cancel_media_index_scan`.

**Reconciliação** (o que faz o índice não apodrecer):
- arquivo no índice mas ausente no disco → `upstream_state` intacto, marca `missing_on_disk`;
- arquivo no disco fora do índice → insere;
- profile root renomeado → atualiza `normalized_path` mantendo o `id` (é o que preserva coleções).

**Pronto quando:** biblioteca real indexada; `load_source_media_gallery` continua
funcionando sem alteração de comportamento; reabrir o app não reindexa o que não mudou.

---

### F1 — Identidade e renomeação de handle (item 13) · tamanho **M**

**Migration `0052_source_identity.sql`**

```sql
CREATE TABLE IF NOT EXISTS identities (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    notes TEXT,
    avatar_source_id TEXT REFERENCES source_profiles(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);

ALTER TABLE source_profiles ADD COLUMN identity_id TEXT REFERENCES identities(id) ON DELETE SET NULL;
ALTER TABLE source_profiles ADD COLUMN provider_user_id TEXT;

CREATE TABLE IF NOT EXISTS source_handle_history (
    source_id TEXT NOT NULL REFERENCES source_profiles(id) ON DELETE CASCADE,
    handle TEXT NOT NULL,
    provider_user_id TEXT,
    first_seen_at TEXT NOT NULL, last_seen_at TEXT NOT NULL,
    PRIMARY KEY (source_id, handle)
);

CREATE INDEX idx_source_profiles_provider_user ON source_profiles(provider, provider_user_id);
```

**Captura do id estável** — cada connector já resolve, só não persiste:
`twitter_connector` → `rest_id` ([twitter_connector.rs:1137](../../src-tauri/src/infrastructure/twitter_connector.rs:1137));
`instagram_connector` → pk / identity hint (já há backfill em [0031](../../src-tauri/migrations/0031_instagram_identity_hint_backfill.sql));
`tiktok_connector` → `secUid`.

**Três detecções no sync:**

1. **Rename** — mesmo `provider_user_id`, handle diferente → registra no histórico, enfileira
   renomeação de pasta via `media_path_migration_runtime`, atualiza `source_profiles.handle`.
   `relative_path` dos ledgers é relativo ao profile root, então **não** precisa reescrita.
2. **Handle reciclado** — mesmo handle, `provider_user_id` diferente → **nunca** renomear e
   **nunca** baixar por cima. Levanta incidente no Workspace Health: alguém pegou o handle
   abandonado e o conteúdo do novo dono iria contaminar o arquivo do antigo. Hoje isso passa
   silencioso e é corrupção de acervo.
3. **Duplicata** — dois sources com o mesmo `(provider, provider_user_id)` → sugere merge.

**Vínculo entre providers:** manual (`link_source_to_identity`) com sugestão automática por
handle idêntico/próximo entre providers. Não inferir sozinho — falso positivo aqui contamina
o dedupe da F5.

**Comandos:** `list_identities`, `create_identity`, `link_source_to_identity`,
`unlink_source_from_identity`, `suggest_identity_links`, `load_source_handle_history`.

**UI:** seção "Identidade" no Source Editor; badge "antes @antigo" no header do ProfileView;
incidente de handle reciclado no Workspace Health com ação "abrir perfil" / "desvincular".

---

### F2 — Removido da origem (item 4) · tamanho **M** · paralelizável com F0

**A armadilha central:** a descoberta do Instagram tem early-stop
([memória: incremental discovery](../../src-tauri/src/infrastructure/instagram_connector.rs)) —
a maioria dos posts não é revisitada em um sync normal. Marcar ausência como remoção
produziria falso positivo em massa. Portanto:

- só avaliar em **`full_scan`** (a opção já existe na Sync tab e no menu de contexto);
- só avaliar se a varredura **terminou sem erro e sem interrupção** (rate limit parcial não conta);
- exigir **N confirmações consecutivas** (default 2) antes de marcar;
- ausência causada por deleção local (`provider_deleted_media`) **não** conta.

**Migration `0053_upstream_state.sql`** — colunas em `provider_sync_post_ledger` **e** no
ledger legado do Instagram (`instagram_sync_post_ledger`), que tem esquema próprio:

```sql
ALTER TABLE provider_sync_post_ledger ADD COLUMN upstream_state TEXT NOT NULL DEFAULT 'present';
ALTER TABLE provider_sync_post_ledger ADD COLUMN missing_confirmations INTEGER NOT NULL DEFAULT 0;
ALTER TABLE provider_sync_post_ledger ADD COLUMN missing_since TEXT;
ALTER TABLE provider_sync_post_ledger ADD COLUMN last_full_scan_at TEXT;

CREATE TABLE IF NOT EXISTS source_full_scan_runs (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES source_profiles(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    completed_cleanly INTEGER NOT NULL DEFAULT 0,
    posts_seen INTEGER NOT NULL DEFAULT 0,
    started_at TEXT NOT NULL, finished_at TEXT
);
```

Propagação para `media_index.upstream_state` via `provider_post_key`.

**UI:** badge "Removido da origem" no card e no lightbox; filtro "Só arquivados aqui" nos
filtros avançados do ProfileView; contador no dashboard. É a feature que muda a percepção
do app de downloader para arquivo — e ela só tem valor se for confiável, daí o rigor acima.

---

### F3 — Timeline agregada (item 8) · tamanho **M** · depende de F0

**Comando:** `load_media_timeline(request) -> MediaTimelinePage`, com **paginação keyset**
(`captured_at DESC, id`) — nunca carregar tudo. O snapshot de bootstrap já trava a UI aos
8,4 MB; o `media_index` **não** entra no snapshot.

**Filtros** (o mesmo `rule_json` das coleções smart): providers, grupos, identidade, seção,
tipo de mídia, período, coleção, `upstream_state`, "só canônicos" (F5), "não vistos".

**Reuso direto:** `MediaCard`, `MediaLightbox`, `lightboxSession.ts`, `thumbnailCache.ts`,
agrupamento por dia do ProfileView e a virtualização já implementada.

**Estado "novidades":** `last_timeline_seen_at` em settings → badge "Novidades (37)". É o que
transforma o acervo em algo que se consome diariamente em vez de algo que se acumula.

---

### F4 — Coleções (item 7) · tamanho **M** · depende de F0, melhor depois de F3

**Migration `0054_collections.sql`**

```sql
CREATE TABLE IF NOT EXISTS collections (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL DEFAULT 'manual',   -- manual | smart
    scope TEXT NOT NULL,                   -- global | source | identity
    scope_ref_id TEXT,
    name TEXT NOT NULL, description TEXT, color TEXT, icon TEXT,
    rule_json TEXT,
    cover_media_id TEXT REFERENCES media_index(id) ON DELETE SET NULL,
    pinned INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS collection_items (
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media_index(id) ON DELETE CASCADE,
    position INTEGER, note TEXT, added_at TEXT NOT NULL,
    PRIMARY KEY (collection_id, media_id)
);

CREATE INDEX idx_collections_scope       ON collections(scope, scope_ref_id, pinned DESC, name);
CREATE INDEX idx_collection_items_media  ON collection_items(media_id);
```

**Comandos:** `list_collections`, `create_collection`, `update_collection`,
`delete_collection`, `promote_collection_to_global`, `add_media_to_collection`
(aceita lote), `remove_media_from_collection`, `load_collection_items`,
`save_filter_as_collection`.

**UI:**
- ProfileView: sidebar com as coleções `scope='source'` + "Nova coleção aqui";
- lightbox: `C` (seletor com a última usada em destaque) e `Shift+C` (adiciona à última);
- seleção múltipla no grid → "Adicionar N itens a…";
- janela Library: aba Coleções com global + identidade, capa, contagem, tamanho em disco;
- botão "Salvar filtro atual como coleção" na timeline (cria `kind='smart'`).

---

### F5 — Dedupe inteligente (item 12) · tamanho **G** · depende de F0 + F1

**Migration `0055_media_variants.sql`**

```sql
CREATE TABLE IF NOT EXISTS media_variant_groups (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,             -- intra_source | cross_source
    identity_id TEXT REFERENCES identities(id) ON DELETE SET NULL,
    canonical_media_id TEXT REFERENCES media_index(id) ON DELETE SET NULL,
    match_kind TEXT NOT NULL,        -- exact_sha256 | perceptual_image | perceptual_video
    confidence REAL NOT NULL,
    policy_applied TEXT NOT NULL,    -- link_only | kept_best | kept_first | keep_all
    reviewed INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS media_variant_members (
    group_id TEXT NOT NULL REFERENCES media_variant_groups(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media_index(id) ON DELETE CASCADE,
    similarity REAL NOT NULL DEFAULT 1.0,
    role TEXT NOT NULL,              -- canonical | variant
    PRIMARY KEY (group_id, media_id)
);
```

**Wave 5a — assinatura de vídeo.** Extensão de `media_metadata_probe`: N frames amostrados
por ffmpeg → dHash por frame → `video_signature` JSON. Custo controlado: só em vídeos
`fingerprint_status='pending'`, na fila do `media_index_runtime`, com perfil de recurso
(reusar `resource_profile` do dedupe).

**Wave 5b — matching.** Busca de candidatos em dois anéis:
- **intra-source, cross-section** (caso story→feed): candidatos = mesmo `source_id`,
  duração ±3%, razão de aspecto compatível;
- **cross-source via identidade** (caso IG+TikTok): candidatos = sources da mesma
  `identity_id`, mesma janela temporal (±7 dias por default).

Sem identidade vinculada, **não** compara cross-source. É o que mantém o custo linear.

**Wave 5c — política e UI.** `link_only` default. Card colapsado com badge de origem
("Instagram + TikTok", "story + feed"), e navegação entre variantes no lightbox. Aba
"Variantes" na janela Library para revisão em lote, com ação de desfazer.

**Ponto de atenção:** `media_dedupe_runtime` (2302 linhas) continua existindo para a
limpeza profunda manual. A F5 não o substitui — alimenta o mesmo vocabulário. Avaliar em
seguida se o VDF por source pode passar a consumir `media_index` em vez de
`media_dedupe_catalog`, eliminando um dos dois catálogos.

---

### F6 — Dashboard do acervo (item 11) · tamanho **M** · depende de F0, F2, F5

**Comando:** `load_library_dashboard(range) -> LibraryDashboard`, agregando `media_index`,
`source_sync_runs`, `source_profiles` e `media_variant_groups`.

**Painéis:**
- volume por provider / perfil / identidade; top 20 perfis por espaço em disco;
- crescimento mensal (itens e bytes);
- taxa de sucesso de sync por provider nos últimos 30 dias e duração média de job;
- **perfis parados, com a distinção que o Health atual não faz:** "sync falhando" vs.
  "o perfil simplesmente não posta há 8 meses" — ações opostas;
- removidos da origem (F2) — quanto do acervo só existe aqui;
- espaço recuperável e duplicatas cross-provider (F5);
- candidatos a arquivar: muito espaço, pouca atividade recente.

Consultas agregadas puras — nada de varredura de disco no caminho de render.

---

## 5. Janela Library

As F3, F4, F5c e F6 vivem numa janela nova, seguindo o padrão multi-janela existente:

```
library.html
src/library.tsx
src/features/library/
  LibraryWindowPage.tsx      abas: Overview | Timeline | Coleções | Variantes
  TimelineView.tsx
  CollectionsPanel.tsx
  VariantReviewPanel.tsx
  DashboardPanel.tsx
  libraryFilters.ts          serialização do rule_json (compartilhado com coleções smart)
```

Registro em `vite.config.ts` (entry), `desktop_runtime.rs` (janela + intent),
`appSections.ts` e `actionRoutes.ts`.

---

## 6. Armadilhas conhecidas

1. **`src/bridge/desktop.ts` tem parsers manuais.** Campo de DTO novo que não for adicionado
   lá **some silenciosamente** — sem erro de compilação e sem erro em runtime. Cada fase que
   cria DTO precisa de item de checklist explícito no parser e teste em `desktop.test.ts`.
2. **Não incluir `media_index` no snapshot de bootstrap.** O snapshot já custa ~3 s por 8,4 MB.
   Toda leitura da F3/F4/F6 é comando dedicado e paginado.
3. **Colisão de migrations entre branches.** O repo já convive com isso via
   `reconcile_colliding_development_migrations` ([database.rs:361](../../src-tauri/src/infrastructure/database.rs:361)).
   As migrations 0051–0055 precisam de reconciliação idempotente equivalente, senão um
   worktree paralelo trava a fila inteira.
4. **Instagram tem ledger próprio.** A F2 precisa cobrir `instagram_sync_post_ledger` além do
   ledger provider-neutro, senão o provider com mais uso fica de fora.
5. **Custo do primeiro índice.** Biblioteca grande = horas de hashing. Mitigado por herdar do
   `media_dedupe_catalog`, mas exige fila com progresso, cancelamento e retomada — nunca
   bloquear a UI nem competir com a fila de download.
6. **Windows:** normalização case-insensitive de path, paths longos e `\` vs `/` em
   `normalized_path`. Já há convenção em `media_dedupe_catalog` — seguir a mesma.
7. **`ON DELETE CASCADE` e o fluxo de deleção existente.** `source_delete_runtime` e a
   deleção de mídia do ProfileView precisam limpar `media_index`, e a remoção em cascata das
   coleções tem que ser intencional (item deletado sai da coleção sem deixar linha órfã).
8. **Falso positivo de identidade contamina o dedupe.** Vínculo entre providers é sempre
   confirmado pelo usuário; sugestão automática nunca aplica sozinha.

---

## 7. Sequência de execução

| Ordem | Fase | Branch sugerida | Bloqueia |
| --- | --- | --- | --- |
| 1 | F0 media_index | `feat/media-index` | F3, F4, F5, F6 |
| 2 | F2 upstream missing | `feat/upstream-missing-detection` | F6 (parcial) |
| 3 | F1 identidade + rename | `feat/source-identity` | F5 cross-provider |
| 4 | F3 timeline | `feat/library-timeline` | F4 (UI) |
| 5 | F4 coleções | `feat/media-collections` | — |
| 6 | F5 dedupe inteligente | `feat/smart-dedupe` | F6 (parcial) |
| 7 | F6 dashboard | `feat/library-dashboard` | — |

F0 e F2 podem correr em paralelo (tocam tabelas distintas). F1 pode entrar em paralelo com
F3/F4 se as migrations forem coordenadas.

**Definição de pronto por fase:** migrations idempotentes e reconciliadas · comandos
registrados em `lib.rs` · parsers em `desktop.ts` com teste · `cargo test` verde (runner com
manifest externo via `.cargo/config.toml`) · `npm run lint` e `npm test` verdes · validação
manual em biblioteca real, registrada aqui.

---

## 8. Entregas parciais que já valem sozinhas

Se a execução for interrompida, estes são os pontos de corte com valor próprio:

- **F0 + F3** → o app ganha um leitor diário do acervo. Maior salto de percepção por esforço.
- **F1 sozinha** → protege contra corrupção por handle reciclado. É defesa, não feature.
- **F2 sozinha** → badge de removido da origem já justifica o app como arquivo.
