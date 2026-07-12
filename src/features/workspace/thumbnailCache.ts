import { convertFileSrc } from '@tauri-apps/api/core'
import type { SourceProfile } from '../../domain/models'
import { loadAvatarThumbnails } from '../../bridge/desktop'

// ----- Module-level state -----

/**
 * sourceId -> asset URL do thumb pequeno em cache local (com `?v=`).
 * Diferente do mecanismo anterior (fetch → blob por avatar, que retinha a
 * imagem em resolução original decodificada no heap do webview), aqui só
 * guardamos URLs: o cache/decodificação ficam com o webview e os arquivos
 * têm ~256px.
 */
const thumbSrcBySource = new Map<string, string>()

/** Incrementado a cada mudança no map; alimenta useSyncExternalStore. */
let epoch = 0

const listeners = new Set<() => void>()

function notify(): void {
  epoch += 1
  for (const listener of listeners) {
    listener()
  }
}

function toAssetUrl(filePath: string): string | undefined {
  if (typeof window === 'undefined') return undefined
  const tauriInternals = (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
  if (!tauriInternals) return undefined
  try {
    return convertFileSrc(filePath)
  } catch {
    return undefined
  }
}

// ----- Public API -----

/**
 * Asset URL que a lista de perfis usa para renderizar o avatar.
 * Prefere o thumb 256px do cache local (nome versionado por mtime, então o
 * path muda quando o avatar troca — cache-buster embutido); cai no arquivo
 * original (volume de mídia) enquanto o lote de thumbs não chega ou quando o
 * original não pôde ser thumbnailado (ex.: gif/bmp). Sem query string: o
 * asset protocol do Windows não a ignora e falharia ao abrir o arquivo.
 */
export function getPreviewSource(source: SourceProfile): string | undefined {
  const thumbSrc = thumbSrcBySource.get(source.id)
  if (thumbSrc) return thumbSrc

  const filePath = source.profileImagePath
  if (!filePath) return undefined
  return toAssetUrl(filePath)
}

/**
 * Pede ao backend para gerar/reaproveitar os thumbs de avatar e atualiza o
 * map local. Sem ids, cobre todos os perfis — barato no lado Rust quando os
 * thumbs estão frescos (stat por arquivo, sem decode). Best-effort: em caso
 * de falha o fallback de getPreviewSource continua renderizando o original.
 */
export async function refreshAvatarThumbnails(sourceIds?: string[]): Promise<void> {
  try {
    const batch = await loadAvatarThumbnails(sourceIds)
    const received = new Map<string, string>()
    for (const thumb of batch.thumbs) {
      // O path já é versionado por mtime ({id}.{mtime}.jpg); a URL muda
      // sozinha quando o avatar troca, sem precisar de query string.
      const assetUrl = toAssetUrl(thumb.path)
      if (!assetUrl) continue
      received.set(thumb.sourceId, assetUrl)
    }

    let changed = false
    if (sourceIds === undefined) {
      // Refresh completo: solta entradas de perfis removidos (ou cujo thumb
      // deixou de gerar) para o fallback voltar a valer.
      for (const key of Array.from(thumbSrcBySource.keys())) {
        if (!received.has(key)) {
          thumbSrcBySource.delete(key)
          changed = true
        }
      }
    }
    for (const [sourceId, src] of received) {
      if (thumbSrcBySource.get(sourceId) !== src) {
        thumbSrcBySource.set(sourceId, src)
        changed = true
      }
    }
    if (changed) notify()
  } catch {
    // Thumbs são otimização; o original ainda renderiza via fallback.
  }
}

/**
 * Invalidate a single source's cached thumbnail.
 * Called after pickSourceProfileImage or resetSourceProfileImage, right
 * before refreshAvatarThumbnails([sourceId]) repopulates it.
 */
export function invalidateSource(sourceId: string): void {
  if (thumbSrcBySource.delete(sourceId)) {
    notify()
  }
}

export function subscribeToAvatarThumbnails(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function getAvatarThumbnailsEpoch(): number {
  return epoch
}
