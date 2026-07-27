# Plano dual (multi-agente): Lightbox media session + Slideshow audio repair

**Status:** execução parcial — **retomar** (Claude session limit)  
**Branch alvo:** `fix/profile-view-lightbox-ux`  
**Base lightbox:** commit `84346c0` — lightbox UX + slideshow audio paths  
**Repair:** working tree / feature na branch (módulo `slideshow_audio_repair`)  
**Criado:** 2026-07-19  
**Agendado para Claude Code:** 2026-07-19 09:21 (America/Sao_Paulo)  
**Modelo:** `fable` · **Effort:** `high` · **Permission:** `auto`  
**Última execução:** 2026-07-19 ~09:22–09:31 -03 · **resultado:** falhou (session limit) com progresso parcial Track A  

---

## Log de execução (2026-07-19 ~15:45 — retomada, Track A COMPLETA)

- **Wave A1 validada:** `lightboxSession.ts` + 24 testes unit verdes. Clamp-on-shrink refeito como derivação (`clampedIndex` em `useMemo`) para satisfazer `react-hooks/set-state-in-effect`; steppers reancoram o índice bruto.
- **Wave A2 — wire completo:**
  - `SingleVideosPage.tsx`: `lightboxIndex`/`videoStartIndices`/`stepLightboxPost|Slide`/`activeVideoPos` locais substituídos por `useLightboxSession` (group key = `video.id`); título do slide via `active.slideIndex/slideCount`.
  - `ProfileViewPage.tsx`: `lightboxGroups`/`findLightboxGroupIndex`/steppers/clamp locais removidos; `openLightboxForPost`, Shift+Del (`deleteActivePost`) e Esc em camadas preservados sobre `lightbox.index`/`open`.
  - `MediaLightbox.tsx`: Space = play/pause (vídeo ou áudio de slideshow); M = mute; volume/mute persistidos (`lightbox.mediaPrefs` no localStorage, reaplicados a cada mount via callback refs + `volumechange`); ←/→ com foco no `<audio>` = seek nativo (não troca slide); debounce de `src` de vídeo (150 ms, 1º vídeo síncrono, vídeo pulado nunca hidrata); fullscreen container + Esc em camadas preservados.
- **Wave A3:** matriz de teclas coberta em `MediaLightbox.test.tsx` (22 testes: transporte, prefs, audio-focus, debounce com fake timers). `tsc -b` limpo, `eslint` limpo nos arquivos da track, suíte completa 718/719 verde — única falha em `.claude/worktrees/single-video-tiktok-story/...` (cópia stale de outro worktree varrida pelo vitest; pré-existente, fora da track).
- **Sem commit/push** (regra). Track B continua pendente (ver seção Track B).

## Log de execução (2026-07-19 ~16:10 — retomada, Track B COMPLETA)

- **B0:** log real `slideshow-audio-repair-20260719-094813.log` analisado (10378 missing, 20× “IP blocked”, ABORT streak=20, recovered=0, rollback total). Padrões do connector mapeados: `enumerate_posts` (impersonate + cookies + UA), `probe_profile_status` (embed page), helpers de sessão já usados pelo repair (`load_account_session_secret_ref` → `parse_session_payload` → `write_netscape_cookie_file`).
- **B1 — redesign em `slideshow_audio_repair.rs` (superado por oEmbed — ver abaixo):**
  - Taxonomia: `inaccessible_or_ip_block` → **`ambiguous_ip_block`**; `deleted_or_missing` → **`post_gone`**.
  - Primeira versão usava `BlockStreakTracker` + probe yt-dlp + rollback de ledger.
- **B2 — contrato/bridge/UI:** `SlideshowAudioRepairResult` com flags de abort / requeue.
- **Sem commit/push** (regra).

## Log de execução (2026-07-19 — Track B evoluiu para oEmbed per-post)

**Fonte da verdade:** `docs/design/slideshow-audio-repair-zero-failures-plan.md`.

Validação empírica mostrou que os 20× “IP blocked” da run real **não eram block de IP** (oEmbed 200 em post vivo + oEmbed 400 nos posts falhos no mesmo IP). Redesign final:

- Confirmação **por post** via TikTok oEmbed (200=alive → requeue; 400=gone → profile embed).
- Profile embed: public → `post_gone`; `10221` → `account_gone`; `10222` → cookie retry forçado → `account_private` se falhar.
- Control post (last recovered / `@scout2015` docs example) distingue oEmbed unreachable local vs rede caida.
- `DownloadPathHealth`: streak de suspect (online mas download falha) → 2× cooldown 180s → abort; **sem rollback** (nada marcado sem evidência).
- Contrato: `abortedOnNetworkBlock` + `requeuedTransient` (posts nunca marcados).
- Pendência restante: **validação real** na conta + possível follow-up de pause longa 15–30 min se aborts por rate-limit forem frequentes.

## Log de execução (2026-07-19 ~09:21)

### Como foi disparado

| Item | Valor |
|------|--------|
| Orquestrador | Grok (task agendada `019f79da67e0`, one-shot) |
| CLI | `claude -p --permission-mode auto --model fable --effort high` |
| Prompt | `Temp/claude-lead-dual-track-prompt.txt` (+ plano como fonte da verdade) |
| Duração | ~9,4 min (~566 s) |
| Exit code | **1** |
| Erro | `You've hit your session limit · resets 2:20pm (America/Sao_Paulo)` |
| Commit/push | não feitos (regra) |

### O que foi feito

#### Track A — parcial (Wave A1 iniciada)

| Entrega | Estado | Notas |
|---------|--------|--------|
| `src/features/workspace/lightboxSession.ts` | **criado** (untracked) | Hook/módulo shared `useLightboxSession`: groups, `open`/`close`, `stepPost`/`stepSlide`, `active` (hasPrev/Next, slideIndex/Count). ~8 KB |
| `src/features/workspace/lightboxSession.test.tsx` | **criado** (untracked) | Suite unit do hook (renderHook). ~11 KB |
| Wire em `SingleVideosPage.tsx` | **iniciado e revertido** | Claude removeu `useState` de `lightboxIndex` e importou o hook, mas **não** reescreveu os usos → página inconsistente. Orquestrador **reverteu** só `SingleVideosPage.tsx` para HEAD para não deixar o frontend quebrado. |
| Wire em `ProfileViewPage.tsx` | **não feito** | — |
| `MediaLightbox.tsx` (Space/M, volume, seek audio-focus, debounce src) | **não feito** | — |
| Waves A2/A3 | **não feitas** | — |

#### Track B — não avançou nesta run

| Entrega | Estado | Notas |
|---------|--------|--------|
| Redesign classificador / probe / circuit breaker | **não feito** | Limit bateu antes |
| Sessão autenticada + probe discriminador | **não feito** | — |
| Testes breaker vs mass deleted | **não feito** | — |
| `slideshow_audio_repair.rs` e painel health | **pré-existentes no working tree** | Código da feature repair/health **já estava** local (stash/WIP); **não** foi o redesign de robustez do plano. Circuit breaker antigo (`streak=20` em `inaccessible_or_ip_block`) permanece. |

#### Outros

- Working tree de health/repair/models/bridge **preservado** (não descartado).
- Plano e prompt em disco mantidos.
- Testes focados da feature dual **não** rodados até o fim.

### Pendências (retomar daqui)

#### Track A — prioridade

1. **Validar** `lightboxSession.ts` + testes (`lightboxSession.test.tsx`) — ajustar API se faltar prefs/hidratação no contrato.
2. **Plugar** `useLightboxSession` em `SingleVideosPage.tsx` (substituir grouping/step local) de forma completa.
3. **Plugar** idem em `ProfileViewPage.tsx`.
4. **MediaLightbox.tsx:**
   - Space = play/pause; M = mute (vídeo + áudio slideshow)
   - volume/mute persistidos entre posts
   - ←/→ com foco em `<audio>` = seek nativo (não troca slide)
   - debounce/lazy de `src` de vídeo em navegação rápida
   - preservar fullscreen container + Esc em camadas
5. Testes UI/regressão matriz de teclas (Wave A3).
6. DoD A completo (ver checklist Track A).

#### Track B — prioridade (ainda crítica)

1. Ler log real:  
   `C:\Users\ninja\AppData\Local\NinjaCrawler\logs\slideshow-audio-repair-20260719-094813.log`  
   (10378 missing; 20× IP blocked → ABORT streak=20; recovered=0).
2. Redesign em `slideshow_audio_repair.rs`:
   - taxonomia: `post_gone` vs `rate_limit` vs block **confirmado**
   - **probe** com sessão autenticada NinjaCrawler (helpers session/cookies) + padrões do connector TikTok
   - mass deleted → mark unavailable e **continuar**
   - block real → circuit breaker inteligente (não abortar só por string yt-dlp)
3. Testes unit classificador + breaker + política de ledger.
4. Bridge/UI só se contadores/copy mudarem.
5. DoD B completo (ver checklist Track B).

#### Orquestração na retomada

1. Preferir após reset do limite Claude (**~14:20** America/Sao_Paulo no dia da falha, ou sessão com quota).
2. Mesmo CLI: `--permission-mode auto --model fable --effort high`.
3. LEAD multi-agente: **não** recriar `lightboxSession` do zero se o módulo atual estiver bom — **continuar** do wire A2 + Track B completa.
4. Prompt de retomada sugerido:  
   “Retome docs/design/lightbox-media-session-plan.md seção Log de execução. Track A: plugar lightboxSession existente + MediaLightbox UX. Track B: robustez repair do zero conforme plano. Não commit/push.”
5. One-shot anterior **não** re-agendado automaticamente.

### Working tree relevante (pós-run)

```
?? src/features/workspace/lightboxSession.ts          # progresso Track A
?? src/features/workspace/lightboxSession.test.tsx    # progresso Track A
?? src-tauri/.../slideshow_audio_repair.rs            # WIP repair (pré-run; sem redesign B)
?? docs/design/lightbox-media-session-plan.md
 M  commands/models/bridge/WorkspaceHealth/styles...  # WIP health/repair UI (pré-run)
```

`SingleVideosPage.tsx` = limpo em relação ao HEAD da branch (parcial revertido).

---

## Visão geral

Duas entregas independentes na **mesma branch**, orquestradas por um **LEAD** com **múltiplos subagentes**, maximizando paralelismo:

| Track | Nome | Área principal | Ownership tipico |
|-------|------|----------------|------------------|
| **A** | Lightbox media session | Frontend React | `MediaLightbox*`, `ProfileView*`, `SingleVideos*`, shared session |
| **B** | Slideshow audio repair robustez | Rust repair + bridge/UI health | `slideshow_audio_repair.rs`, commands/models repair, `WorkspaceHealth*` só se necessário |

**Regra:** tracks A e B **não compartilham arquivos quentes** → podem avançar em paralelo sob o mesmo Lead.  
**Proibido:** commit/push; reset destrutivo; misturar refactors cosméticos.

---

# TRACK A — Unificar media session do lightbox

## Objetivo A

Unificar navegação, áudio de slideshow no player, volume/mute e hidratação de mídia — modelo compartilhado entre Profile View e Single Videos, sem regressões do `84346c0`.

## Contexto A (`84346c0`)

- Fullscreen no container; Enter; Esc em camadas
- ↑/↓ = post; ←/→ = slide ou seek de vídeo
- Meta (views, índice); `audio_*_path` no gallery
- Duplicação de grouping/step entre PV e SV; volume não persiste; Space/M fracos; colisão seek/`<audio>`; risco OOM em nav rápida

## Escopo A

| Capacidade | Esperado |
|------------|----------|
| Navegação | post ↑/↓; slide ←/→; seek vídeo; seek nativo se foco no `<audio>` |
| Transporte | Space play/pause; M mute (vídeo e áudio slideshow) |
| Prefs | volume/mute persistidos (sessão e/ou localStorage) |
| Hidratação | debounce/lazy de `src` de vídeo |
| Fullscreen | preservar container + Esc em camadas |
| API | PV e SV usam o mesmo módulo de sessão/navegação |

### Fora de escopo A

- Backend Rust (preferir não tocar)
- Redesign visual grande; preload de rede
- WorkspaceHealth / repair (isso é Track B)

## Waves multi-agente — Track A

```text
Wave A0 (//) Explore MediaLB | PV | SV | testes
     ↓
Wave A1      Core shared API + unit tests  [sequencial]
     ↓
Wave A2 (//) Host-PV | Host-SV | Lightbox-UX
     ↓
Wave A3 (//) Tests-A | Reviewer-A
```

| Wave | Agentes | Ownership |
|------|---------|-----------|
| A0 | 4 explores RO | inventário |
| A1 | Core | **novo** `lightboxSession.ts` (ou nome alinhado) + unit |
| A2 | Host-PV / Host-SV / Lightbox-UX | `ProfileViewPage*`, `SingleVideosPage*`, `MediaLightbox*` — **1 arquivo-dono** |
| A3 | Tests-A, Reviewer-A | testes + diff A |

### Matriz de teclas A

| Tecla | Contexto | Ação |
|-------|----------|------|
| ↑ / ↓ | aberto | post ±1 |
| ← / → | foto, foco ≠ audio | slide ±1 |
| ← / → | vídeo | seek ±1s |
| ← / → | foco em `<audio>` | seek nativo |
| Enter | | toggle FS container |
| Esc | FS / não FS | exit FS / close |
| Space / M | mídia | play-pause / mute |

### DoD A

- [x] Módulo shared de sessão/navegação criado (`lightboxSession.ts` + testes unit)  
- [x] PV e SV na mesma API de sessão/navegação (wire completo em `ProfileViewPage.tsx` e `SingleVideosPage.tsx`)  
- [x] Matriz crítica coberta por testes (Space/M, setas, audio-focus, debounce, FS/Esc em `MediaLightbox.test.tsx`; regressão hosts verde)  
- [x] Volume/mute herdados entre posts (localStorage `lightbox.mediaPrefs`, reaplicado em cada mount de mídia)  
- [x] Debounce de hidratação documentado (`VIDEO_HYDRATE_DEBOUNCE_MS = 150 ms` em `MediaLightbox.tsx`; unmount imediato do vídeo antigo, src novo só após settle; 1º vídeo síncrono)  
- [x] Sem regressão audio path / multi-image / FS / Esc (suíte `MediaLightbox` + `ProfileViewPage` verde)  
- [x] Testes focados verdes; sem commit/push  

---

# TRACK B — Slideshow audio repair: robustez (deleted vs block)

## Objetivo B

Tornar o **one-shot slideshow audio repair** robusto quando **muitos posts foram deletados pelo creator** e o TikTok devolve o erro enganoso **“Your IP address is blocked from accessing this post”**.

Hoje a run **aborta cedo** por streak de “block”, desfaz o ledger e deixa **milhares** ainda na fila — mesmo quando o problema real é **conteúdo offline/deletado**, não um IP block global.

A solução deve ser a **mais correta tecnicamente**:

1. Reutilizar a **sessão autenticada do NinjaCrawler** (cookies/secret de conta já no app).  
2. Validar com os **padrões do connector TikTok** do projeto (não reinventar auth ad hoc).  
3. **Discriminar** falha de post (deleted/private/unavailable) vs bloqueio real de rede/IP/rate-limit.  
4. Em massa de deleted: **marcar unavailable e continuar** a fila.  
5. Em block real: **circuit breaker inteligente** (probe, backoff, pause) — não confundir com deleted.

## Evidência da última run (obrigatório ler)

**Log:**  
`C:\Users\ninja\AppData\Local\NinjaCrawler\logs\slideshow-audio-repair-20260719-094813.log`

Fatos do header/footer:

| Campo | Valor |
|-------|--------|
| mode | inline one-shot (NOT source-sync queue) |
| tracks_to_download | **10378** |
| jobs_with_cookies | 10378/10378 |
| policy | one_attempt_per_post; failures → unavailable |
| nota no log | TikTok often reports deleted as “IP address is blocked” |
| class observada | `inaccessible_or_ip_block` (20×) |
| ABORT | `streak=20 threshold=20` — suspected real IP block |
| recovered | **0** |
| remaining_missing | **10378** |
| aborted_on_suspected_block | **true** |
| ledger | rollback dos 20; nada marcado unavailable |

Trecho conceitual do código atual (`slideshow_audio_repair.rs`):

- `CONSECUTIVE_BLOCK_ABORT_THRESHOLD = 20`
- `classify_tiktok_audio_error`: string `"ip address is blocked"` → `inaccessible_or_ip_block`
- Circuit breaker: streak de block/rate_limit → **abort + rollback ledger**
- Comentário no código já admite: TikTok **mislabels deleted** como IP blocked

## Problema de desenho (o que corrigir)

```text
yt-dlp FAIL "IP blocked"
        │
        ▼
 classify → inaccessible_or_ip_block
        │
        ▼
 mark unavailable + increment streak
        │
        ▼
 streak >= 20  ──► ABORT run + rollback streak
                   (assume "real IP block")
```

Em bibliotecas grandes com muitos posts apagados, **20 “IP blocked” seguidos é o caso normal de deleted**, não prova de block global. O circuit breaker atual é **conservador demais na direção errada** (protege contra mass-mark, mas **impede** mass-mark legítimo de deleted e **mata** runs de 10k).

## Direção técnica correta (contrato de robustez)

O Lead/Core-B deve **validar no código real** (connector, session store, likes runtime, etc.) e escolher o desenho mais alinhado ao repo. Diretrizes obrigatórias:

### B1. Taxonomia de falha (não confiar só na string do yt-dlp)

Separar explicitamente classes estáveis, por exemplo:

| Classe | Significado | Ação na fila |
|--------|-------------|--------------|
| `post_gone` / deleted / not found | post removido ou inacessível **por conteúdo** | mark unavailable; **reset ou não contar** streak de block |
| `private_or_auth` | precisa sessão / sem permissão | retry com cookies se ainda não usou; senão unavailable ou skip classificado |
| `rate_limit` | 429 / rate | backoff; conta para circuit de rede |
| `ip_or_network_block` | block **confirmado** (probe falhou em post conhecido-bom) | pause/abort **com** política clara; **não** mass-mark |
| `extractor_error` / unknown | outros | política explícita + telemetria |

### B2. Sonda de sanidade com sessão autenticada (discriminador)

Antes de abortar por “IP block”, ou periodicamente durante streak:

1. Carregar sessão da conta NinjaCrawler (`load_account_session*` / secret store — **reusar** helpers existentes).  
2. Usar o **mesmo caminho de cookies/UA** que o connector TikTok já usa com sucesso no app (não inventar Netscape file se já existe abstração).  
3. Fazer **probe** de um alvo de controle, preferindo nesta ordem:
   - post/slideshow **ainda online** do mesmo creator (se conhecida);
   - endpoint/perfil ou URL de controle que o connector já sabe validar com sessão;
   - último post **recovered com sucesso** nesta run (se houver).  
4. Interpretação:
   - probe **OK** + fail no post alvo → tratar alvo como **post_gone / inaccessible terminal** (mark unavailable, **continuar**).  
   - probe **também** “IP blocked” / network fail → **block real** → circuit breaker (pause, backoff, abort controlado **sem** marcar milhares de posts de uma vez).

### B3. Circuit breaker redesenhado

- Streak de **apenas** falhas de rede/block **confirmadas** (ou rate_limit), não de “IP blocked” bruto do yt-dlp.  
- Streak de post_gone **não aborta** a run.  
- Opcional: janela deslizante (ex. N fails / M attempts) + amostragem de probe a cada K falhas ambíguas.  
- Em abort real: persistir progresso (recovered/unavailable já commitados); **não** rollback de post_gone legítimos.  
- Telemetria no log: `class`, `probe_result`, `streak_network`, `streak_post_gone`, `cookies_used`, `account_id`.

### B4. Integração com connector / sessão (não bypass frágil)

- **Ler** como o TikTok connector e o Single Videos baixam áudio/photo-mode e como montam cookies.  
- Repair deve **convergir** para essas convenções (impersonate, UA, cookie timing, `/video/` vs `/photo/`).  
- Se cookies “quebram universal data” (comentário atual), documentar e manter attempt order **testado**, mas a **classificação** pós-erro usa probe autenticado.  
- Nunca logar secrets/cookies em claro.

### B5. UX / painel (se necessário)

- Progresso: recovered / unavailable(post_gone) / network_abort / remaining.  
- Mensagens honestas: “posts removidos marcados” vs “run pausada por bloqueio de rede”.  
- Manter clear unavailable ledger.  
- Só tocar UI de health se o contrato de progresso mudar.

### B6. Testes

- Unit: classificador (strings reais do log).  
- Unit/integration: circuit breaker **não** aborta em 20× IP-blocked quando probe diz “rede OK”.  
- Abort **só** quando probe confirma block.  
- Rollback policy: o que entra/sai do ledger.  
- Não precisa baixar 10k posts em CI — mocks de download/probe.

## Escopo B

### Dentro

- `src-tauri/src/infrastructure/workspace_repository/slideshow_audio_repair.rs` (+ testes no mesmo módulo ou tests.rs)  
- Models/commands/bridge se o resultado/progress precisar de campos novos (`aborted`, contagens por classe, etc.)  
- UI mínima se contadores/copy enganarem o usuário  
- Alinhar auth/probe com sessão NinjaCrawler + padrões do connector  

### Fora de escopo B

- Reescrever o sync queue completo  
- Mudar política global de rate-limit do profile sync (salvo reuso de helpers)  
- Track A (lightbox)  
- Commit/push  

## Waves multi-agente — Track B

```text
Wave B0 (//) Explore repair.rs | session/cookies helpers | tiktok connector audio path | log real
     ↓
Wave B1      Core-B: taxonomia + probe + circuit breaker  [sequencial no repair.rs]
     ↓
Wave B2 (//) Tests-B | UI/bridge (se contrato mudou) | Reviewer-B
     ↓
Wave B3      (opcional) smoke mental / doc de validação manual com conta real
```

| Wave | Agentes | Foco |
|------|---------|------|
| B0-Explore-Repair | RO | `slideshow_audio_repair.rs` loop, abort, classify, download |
| B0-Explore-Session | RO | `load_account_session*`, `write_netscape_cookie_file`, secret store |
| B0-Explore-Connector | RO | TikTok connector / single-video audio path, impersonate, cookies |
| B0-Explore-Log | RO | log `slideshow-audio-repair-20260719-094813.log` — padrões, falsos positivos |
| B1-Core | implement | **só** `slideshow_audio_repair.rs` (+ testes no módulo) — redesign class/probe/breaker |
| B2-BridgeUI | implement | models/commands/desktop/health **apenas se** API pública mudar |
| B2-Tests | implement | casos classificador + breaker + ledger |
| B2-Reviewer | RO | DoD B, segurança de secrets, não regredir recovered path |

**Ownership:** Wave B1 é o gargalo (um arquivo principal). Não spawnar dois writers em `slideshow_audio_repair.rs` ao mesmo tempo.

## DoD B

- [x] Log real da run analisado e referenciado no raciocínio (string real do log usada nos testes do classificador)  
- [x] “IP blocked” **ambíguo** não incrementa sozinho o abort de rede (classe `ambiguous_ip_block`; abort exige probe)  
- [x] Probe com **sessão autenticada NinjaCrawler** (cookies/UA do `AccountAuthBundle` já derivado dos helpers de sessão) discrimina post_gone vs block  
- [x] Massa de posts deletados: mark unavailable + **run continua** (probe network_ok mantém as marcas e segue; threshold dobra até 320)  
- [x] Block real: abort/pause controlado **sem** mass-mark incorreto e **sem** apagar progresso bom (rollback só da streak não provada; `requeued_after_abort`)  
- [x] Log/progresso distinguem classes (`class=` por FAIL, linhas `PROBE target/verdict/detail`, footer com `probes_network_ok` / `aborted_on_network_block` / `requeued_after_abort`)  
- [x] Testes unit do classificador + breaker (16 testes no módulo, incluindo política de ledger)  
- [x] Sem vazar cookies/secrets (log imprime só `cookies=true(count)` e URLs públicas)  
- [x] Código alinhado a padrões do connector (flags do `enumerate_posts`: impersonate chrome, `--no-cookies-from-browser`, cookies+UA; probe de perfil espelha o listing)  
- [x] Sem commit/push  

---

# Orquestração global (LEAD)

## Topologia

```text
                         LEAD (fable / high / auto)
                    ┌─────────────┴─────────────┐
                    ▼                           ▼
              TRACK A (//)                 TRACK B (//)
           waves A0→A3                   waves B0→B2
         (frontend session)            (repair Rust + probe)
                    └─────────────┬─────────────┘
                                  ▼
                         Integração + DoD A+B
                         git status / diff --stat
                         resumo PT-BR
```

## Paralelismo máximo recomendado

1. **Início:** spawna **A0 (4)** e **B0 (4)** em paralelo (tudo read-only).  
2. Em seguida, **A1** e **B1** em paralelo (arquivos diferentes).  
3. Depois **A2 (3)** // **B2 (bridge/tests/review)** conforme ownership.  
4. Lead só serializa se houver conflito de arquivo (ex. `models.rs` / `desktop.ts` tocados por B2 e algo de A — **evitar**: A não deve tocar models Rust; B só se necessário).

## Conflitos a evitar

| Arquivo | Preferência |
|---------|-------------|
| `MediaLightbox*`, `ProfileView*`, `SingleVideos*`, `lightboxSession*` | **só Track A** |
| `slideshow_audio_repair.rs` | **só Track B** |
| `domain/models.rs`, `commands.rs`, `desktop.ts`, `WorkspaceHealth*` | Track B se API repair mudar; A **não** toca |
| Working tree alheio não-repair/não-lightbox | **não tocar** |

## Prompt operacional (Claude Code CLI)

```text
Você é o LEAD orquestrador. Há DUAS tracks. Use MÚLTIPLOS SUBAGENTES em
paralelo (Task/subagents). Não faça tudo sozinho.

Plano (fonte da verdade):
docs/design/lightbox-media-session-plan.md

Flags de contexto: --permission-mode auto --model fable --effort high

## Regras duras
- NÃO commit/push; NÃO git reset destrutivo
- Track A = lightbox media session (frontend)
- Track B = slideshow audio repair robustez (Rust + sessão autenticada)
- Ownership por arquivo; tracks A e B em paralelo
- Responder em português no final
- NÃO logar cookies/secrets

## TRACK A — resumido
Waves A0 explore // → A1 shared session API → A2 PV|SV|MediaLightbox // → A3 tests/review
DoD: API única, Space/M, volume persistido, seek com foco audio, debounce video src, FS/Esc ok

## TRACK B — resumido (CRÍTICO)
Log obrigatório:
C:\Users\ninja\AppData\Local\NinjaCrawler\logs\slideshow-audio-repair-20260719-094813.log

Problema: 10378 missing; 20× "IP address is blocked" classificados como
inaccessible_or_ip_block; ABORT streak=20; recovered=0; rollback ledger.
Muitos posts deletados pelo creator; TikTok manda "IP blocked" falso.

Exigências:
1) Discriminar post deletado/indisponível vs block/rate-limit REAL
2) Usar sessão autenticada do NinjaCrawler (helpers de account session/cookies)
   e padrões do connector TikTok do repo para validação/probe — solução
   tecnicamente correta e robusta, best practices do codebase
3) Massa de deleted: mark unavailable e CONTINUAR a run
4) Block real: circuit breaker inteligente com probe; não abortar só por
   string "IP blocked" do yt-dlp
5) Testes unit classificador + breaker; telemetria no log por classe
6) Código em slideshow_audio_repair.rs (+ models/UI só se preciso)

Waves B0 explore // (repair, session, connector, log) → B1 core repair →
B2 tests|bridge|review //

## Ordem de spawn sugerida
1. Em paralelo: todos explores A0 + B0
2. Em paralelo: Core A1 + Core B1
3. Em paralelo: implementers A2 + B2
4. Lead: testes focados, DoD A+B, git diff --stat, resumo

Se um subagente falhar, reatribua o gap; não reinicie as duas tracks do zero.
```

### Invocação

```powershell
claude -p --permission-mode auto --model fable --effort high "<prompt operacional acima>"
```

---

## Notas de agenda (Grok)

- Disparo: **2026-07-19 09:21** America/Sao_Paulo  
- Resultado: **falhou** por session limit Claude (~09:31); ver **Log de execução** no topo  
- CLI: `--permission-mode auto --model fable --effort high`  
- Plano dual multi-agente = fonte da verdade (+ estado de progresso/pendências)  
- Log de repair acima = evidência obrigatória da Track B  
- Retomar manualmente após quota; one-shot **não** re-agendado 
