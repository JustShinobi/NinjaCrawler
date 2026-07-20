# Plano: Slideshow audio repair — rumo a zero falhas

**Origem:** follow-up da Track B de `lightbox-media-session-plan.md`.
**Objetivo:** uma run completa da fila (~10k) terminar sem intervenção manual, com falha apenas em mídia **comprovadamente** indisponível.
**Status código (2026-07-19):** árvore oEmbed per-post + `DownloadPathHealth` **implementada** em `slideshow_audio_repair.rs`. O redesign B1 (BlockStreakTracker + yt-dlp probe + rollback de ledger) foi **substituído** por confirmação determinística por post — nada é marcado no ledger sem evidência, então aborts não fazem rollback.

**UI:** painel Workspace Health removido. Entry point suportado = CLI headless  
`cargo run -p ninjacrawler --bin slideshow_audio_repair_cli -- […]`  
(scan/download não expostos na UI; `state.json` + logs sob `%LOCALAPPDATA%\NinjaCrawler`).

### Telemetria de log (desenho atual)

- `FAIL … class=…` + `confirm: oembed=alive|gone|unreachable…` + `action=requeued|marked_unavailable`
- `COOLDOWN … cycle N/2` (path suspect: posts online mas download falha)
- `ABORT confirmed_by=oembed_alive_streak|control_probe`
- Footer: `confirmed_gone`, `requeued_transient`, `recovered_via_cookie_retry`, `cooldowns_used`, `aborted_on_network_block`

## Pendências mapeadas (atualizado 2026-07-19)

1. **Validação real do discriminador (obrigatória antes de confiar).**
   Rodar o repair com a conta e observar as linhas `confirm: oembed=…` no log
   (`slideshow-audio-repair-*.log`). Esperado nos primeiros ~100 FAIL:
   - `oembed=alive … action=requeued` (não grava ledger)
   - `oembed=gone profile=public … action=marked_unavailable` (`post_gone`)
   - `oembed=gone profile=private cookie_retry=…` quando há auth
   - ausência de mass-mark de `ambiguous_ip_block` no ledger

2. **Retomada automática pós-block (parcial).**
   Implementado: streak de suspect (threshold 8) → até **2 cooldowns de 180s**
   → abort controlado com fila preservada (posts requeued nunca foram
   marcados). **Não** implementado: pause de 15–30 min com auto-resume
   (congelaria o painel sem cancel token). Follow-up se validação real
   mostrar aborts frequentes por rate-limit prolongado.

3. **Classes residuais a monitorar no log.**
   - `unknown` / `extractor_error`: passam por oEmbed (ambiguous set);
     se oEmbed alive → requeue; se gone → mark. Volume alto de unknown com
     oEmbed alive = classificador yt-dlp precisa aprender strings novas.
   - `photo_no_av_format`: **não marca unavailable**. Exige oEmbed.
     Alive → **requeue** (post ainda precisa de repair; trilha não extrável
     *agora* — yt-dlp/TikTok podem voltar a expor; não conta como path-block).
     Gone → profile/cookie path.
     **Não confundir com Single Videos:** o single download de photo baixa as
     **imagens** do carrossel (rehydration `imagePost`); o áudio é best-effort
     e costuma falhar em silêncio. O repair só grava `{post_id}_audio.*`.
     Ex.: `@2julinda/7223126054590762245` — oEmbed 200, fotos no Single Videos,
     `music.playUrl=""`, yt-dlp sem formats → requeue (não é “post morto”).
   - `account_private` com cookie_retry fail → **requeue** (não abandonar);
     mark só `account_gone` / `post_gone` com evidência de sumiço.
   - `private_or_auth` terminal no classificador; o caminho oEmbed+10222
     força cookie retry mesmo quando yt-dlp disse "IP blocked" / "no formats".

4. **Edge de handle renomeado.**
   URLs usam o handle salvo no scan; `/@handle-antigo/video/{id}` costuma
   redirecionar pelo post-id, mas se o TikTok parar de redirecionar vira
   `post_gone` incorreto. Raro; re-scan pega o handle atual do DB.

5. **Validação incremental antes da run completa.**
   Limpar ledger se necessário; observar primeiros ~100 posts no log com
   a telemetria acima. Só então confiar na fila ~10k.

## Confirmação determinística de post indisponível — VALIDADO (2026-07-19)

Teste real feito do mesmo IP da run que abortou com 20× "IP address is
blocked" (curl, sem cookies):

| Alvo | Endpoint | Resultado |
|------|----------|-----------|
| Post **vivo** (`@scout2015/video/6718335390845095173`, exemplo da doc) | `https://www.tiktok.com/oembed?url=<post>` | **HTTP 200** + JSON completo (title, author, html) |
| Post do log `@027_araujo/…/7588998641923230997` (yt-dlp: "IP blocked") | oEmbed | **HTTP 400** `{"message":"Something went wrong","code":400}` |
| Post do log `@1helenna/…/7238007550753492229` (yt-dlp: "IP blocked") | oEmbed | **HTTP 400** idem |
| Perfil `@027_araujo` | `https://www.tiktok.com/embed/@handle` | **HTTP 200** (perfil público existe → posts falhos foram **deletados individualmente**) |
| Perfil `@1helenna` | embed | **HTTP 400** + `"errorCode":10222` (**conta ficou privada** — não deletada!) |

Conclusões:

1. **O "IP blocked" do yt-dlp naquela run NÃO era block de IP** — provado: o
   mesmo IP recebia 200 para post vivo e 400 para os posts falhos, no mesmo
   instante.
2. **oEmbed dá evidência estruturada POR POST** (não por streak): vivo → 200
   com metadados; indisponível → 400. Sem auth, GET simples, barato.
3. **O embed de perfil (padrão já usado pelo connector) refina a causa**:
   200 público → post deletado (`post_gone` certo); `errorCode:10222` →
   conta privada; `errorCode:10221` → conta inexistente/banida.
4. **Caso @1helenna revela posts recuperáveis hoje perdidos**: conta privada
   + sessão que a segue ⇒ o download com cookies poderia funcionar, mas o
   fallback de cookies só dispara em `private_or_auth` — e o TikTok responde
   "IP blocked", então o fallback nunca roda. Com o embed dizendo "privada",
   dá para forçar a tentativa com cookies antes de marcar unavailable.

### Árvore de decisão proposta (por post que falhar como `ambiguous_ip_block`)

```text
download FAIL "IP blocked"
  → oEmbed do post
      200 → post EXISTE público → falha foi rede/extractor → REQUEUE (nunca marcar)
      400 → embed do perfil
              200 público            → post deletado → unavailable (certeza)
              errorCode 10221        → conta sumiu   → unavailable (certeza)
              errorCode 10222 privada→ tem cookies? retry download com cookies
                                        OK  → recovered
                                        FAIL→ unavailable (privado sem acesso)
      falha de rede/timeout no oEmbed
          → controle (oEmbed de post vivo conhecido / último recovered)
              controle 200 → 400 do alvo é confiável
              controle falha → block real de rede → pause/abort
```

### Limites da certeza (por honestidade)

- 100% absoluto não existe (o servidor pode responder WAF/transientes), mas o
  esquema acima exige que o TikTok responda 400 ao alvo E 200 ao controle no
  mesmo instante para gerar um falso "unavailable" — os casos restantes
  (privado, region-lock) são de fato não-baixáveis deste ambiente, e o caso
  "privado com acesso" é coberto pelo retry com cookies.
- Validar com um post de FOTO vivo (testamos deletados na forma `/video/`;
  o vivo testado era vídeo). O primeiro recovered da própria run serve.
- oEmbed em volume (10k) precisa de throttle próprio; usar só para posts que
  falharam, com o controle amortizado.
