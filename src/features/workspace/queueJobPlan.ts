import type {
  ProviderKey,
  SourceSyncOptions,
  SourceSyncOptionsOverride,
} from '../../domain/models'
import { resolveSyncSectionChipsFor, type SyncSectionChip } from './profileSyncSections'

/**
 * De onde o job veio. Derivado do `trigger` cru do backend, que é texto livre
 * (`manual`, `manual_force_imported_backfill`, `companion`, …).
 */
export type QueueJobOrigin = 'grid' | 'companion' | 'scheduler' | 'plan' | 'retry' | 'unknown'

/**
 * O que o job cobre. `single_story` / `single_video` são alvos pontuais enviados
 * pelo Companion: neles a config de sections do perfil é irrelevante, porque só
 * um item é baixado.
 */
export type QueueJobScope = 'profile' | 'single_story' | 'single_video'

export interface QueueJobPlan {
  origin: QueueJobOrigin
  originLabel: string
  scope: QueueJobScope
  /** Rótulo curto do alvo pontual (id do story, URL do vídeo). */
  targetLabel?: string
  /**
   * Trilha de sections da configuração *efetiva* (perfil + override). Vazia para
   * escopos pontuais e para providers sem trilha definida (YouTube, VSCO).
   */
  sections: SyncSectionChip[]
  /** Modificadores do job: full scan, missing only, backfill, janela de datas. */
  notes: string[]
  /** Tooltip agregado da linha. */
  summary: string
}

interface QueueJobPlanInput {
  provider: ProviderKey
  trigger?: string
  runMode?: string
  syncOptionsOverride?: SourceSyncOptionsOverride
  /** Config salva no perfil; ausente quando o perfil não está na referência. */
  profileSyncOptions?: SourceSyncOptions
}

const ORIGIN_LABELS: Record<QueueJobOrigin, string> = {
  grid: 'Grid',
  companion: 'Companion',
  scheduler: 'Scheduler',
  plan: 'Plan',
  retry: 'Retry',
  unknown: 'Queued',
}

function resolveOrigin(trigger: string | undefined): QueueJobOrigin {
  const value = trigger?.trim().toLowerCase()
  if (!value) {
    return 'unknown'
  }
  // O Companion enfileira como `chrome_extension` / `chrome_extension_story`;
  // `companion` é aceito por robustez, mas não é o valor emitido hoje.
  if (value.includes('chrome_extension') || value.includes('companion')) return 'companion'
  if (value.includes('scheduler') || value.includes('schedule')) return 'scheduler'
  if (value.includes('plan')) return 'plan'
  if (value.includes('retry')) return 'retry'
  if (value.includes('manual')) return 'grid'
  return 'unknown'
}

function trimmed(value: string | undefined): string | undefined {
  const next = value?.trim()
  return next && next.length > 0 ? next : undefined
}

/** Encurta a URL de um alvo pontual para caber na headline. */
function shortenTarget(value: string): string {
  const withoutQuery = value.split('?')[0]!.replace(/\/+$/, '')
  const lastSegment = withoutQuery.split('/').pop()
  return lastSegment && lastSegment.length > 0 ? lastSegment : withoutQuery
}

/**
 * Config efetiva do job: o override vence campo a campo, e o que ele não define
 * vem do perfil. A ausência de uma chave no override significa "herda", então o
 * merge é raso e por provider.
 */
function mergeEffectiveOptions(
  provider: ProviderKey,
  profileSyncOptions: SourceSyncOptions | undefined,
  override: SourceSyncOptionsOverride | undefined,
): SourceSyncOptions | undefined {
  if (!override) {
    return profileSyncOptions
  }

  switch (provider) {
    case 'instagram':
      return {
        ...profileSyncOptions,
        instagram: {
          ...(profileSyncOptions?.instagram ?? {}),
          ...override.instagram,
        } as SourceSyncOptions['instagram'],
      }
    case 'tiktok':
      return {
        ...profileSyncOptions,
        tiktok: { ...(profileSyncOptions?.tiktok ?? {}), ...override.tiktok },
      }
    case 'twitter':
      return {
        ...profileSyncOptions,
        twitter: { ...(profileSyncOptions?.twitter ?? {}), ...override.twitter },
      }
    default:
      return profileSyncOptions
  }
}

function collectNotes(effective: SourceSyncOptions | undefined, runMode: string | undefined): string[] {
  const notes: string[] = []
  const instagram = effective?.instagram

  if (instagram?.fullScan) notes.push('Full scan')
  if (instagram?.missingOnly) notes.push('Missing only')

  const dateFrom = trimmed(instagram?.dateFrom)
  const dateTo = trimmed(instagram?.dateTo)
  if (dateFrom || dateTo) {
    notes.push(`Dates ${dateFrom ?? '…'} → ${dateTo ?? '…'}`)
  }

  const mode = trimmed(runMode)
  if (mode) {
    notes.push(mode.replaceAll('_', ' '))
  }

  return notes
}

export function resolveQueueJobPlan({
  provider,
  trigger,
  runMode,
  syncOptionsOverride,
  profileSyncOptions,
}: QueueJobPlanInput): QueueJobPlan {
  const origin = resolveOrigin(trigger)
  const effective = mergeEffectiveOptions(provider, profileSyncOptions, syncOptionsOverride)

  const storyMediaId = trimmed(syncOptionsOverride?.instagram?.targetStoryMediaId)
  const videoUrl = trimmed(syncOptionsOverride?.tiktok?.targetVideoUrl)

  if (storyMediaId || videoUrl) {
    const scope: QueueJobScope = storyMediaId ? 'single_story' : 'single_video'
    const rawTarget = storyMediaId ?? videoUrl!
    const targetLabel = storyMediaId ? storyMediaId : shortenTarget(rawTarget)
    const scopeLabel = scope === 'single_story' ? '1 story' : '1 video'
    return {
      origin,
      originLabel: ORIGIN_LABELS[origin],
      scope,
      targetLabel,
      sections: [],
      notes: [],
      summary: `${scopeLabel} · ${rawTarget}`,
    }
  }

  const sections = resolveSyncSectionChipsFor(provider, effective)
  const notes = collectNotes(effective, runMode)
  const enabled = sections.filter((chip) => chip.enabled).map((chip) => chip.label)

  const summaryParts = [`Origin: ${ORIGIN_LABELS[origin]}`]
  if (sections.length > 0) {
    summaryParts.push(
      enabled.length > 0 ? `Sections: ${enabled.join(', ')}` : 'No sync sections enabled',
    )
  }
  summaryParts.push(...notes)

  return {
    origin,
    originLabel: ORIGIN_LABELS[origin],
    scope: 'profile',
    sections,
    notes,
    summary: summaryParts.join(' · '),
  }
}
