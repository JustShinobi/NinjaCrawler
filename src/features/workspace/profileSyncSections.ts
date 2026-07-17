import type { SourceProfile } from '../../domain/models'
import {
  createInstagramSourceSyncOptions,
  createTikTokSourceSyncOptions,
  createTwitterSourceSyncOptions,
} from '../../domain/sourceSyncOptions'

// Um "chip" da trilha de sections do card. `code` é o rótulo curto (2 chars)
// mostrado no grid; `label` é o nome completo usado no tooltip. A ordem do
// array é fixa por provider — a posição do chip carrega significado, então
// nunca reordene sem atualizar a legenda.
export interface SyncSectionChip {
  code: string
  label: string
  enabled: boolean
}

// Reflete a configuração efetiva: quando `syncOptions` está ausente, as
// funções create* preenchem os defaults do provider (ex.: Instagram timeline
// ligada, resto desligado), então o fingerprint mostra o que de fato roda.
export function resolveSyncSectionChips(source: SourceProfile): SyncSectionChip[] {
  switch (source.provider) {
    case 'instagram': {
      const options = createInstagramSourceSyncOptions(source.syncOptions?.instagram)
      return [
        { code: 'TL', label: 'Timeline', enabled: options.timeline },
        { code: 'RE', label: 'Reels', enabled: options.reels },
        { code: 'ST', label: 'Stories', enabled: options.stories },
        { code: 'SU', label: 'Stories (user)', enabled: options.storiesUser },
        { code: 'TG', label: 'Tagged', enabled: options.tagged },
      ]
    }
    case 'tiktok': {
      const options = createTikTokSourceSyncOptions(source.syncOptions?.tiktok)
      return [
        { code: 'TL', label: 'Timeline', enabled: Boolean(options.getTimeline) },
        { code: 'US', label: 'User stories', enabled: Boolean(options.getStoriesUser) },
        { code: 'RP', label: 'Reposts', enabled: Boolean(options.getReposts) },
        { code: 'LK', label: 'Liked', enabled: Boolean(options.getLikedVideos) },
      ]
    }
    case 'twitter': {
      const options = createTwitterSourceSyncOptions(source.syncOptions?.twitter)
      return [
        { code: 'MD', label: 'Media', enabled: Boolean(options.mediaModel) },
        { code: 'PR', label: 'Profile', enabled: Boolean(options.profileModel) },
        { code: 'SE', label: 'Search', enabled: Boolean(options.searchModel) },
        { code: 'LK', label: 'Likes', enabled: Boolean(options.likesModel) },
      ]
    }
    default:
      return []
  }
}

// Tooltip agregado da trilha: "Timeline, Reels" (só as habilitadas) ou um
// aviso quando nenhuma section está ativa.
export function summarizeEnabledSections(chips: SyncSectionChip[]): string {
  const enabled = chips.filter((chip) => chip.enabled).map((chip) => chip.label)
  if (enabled.length === 0) {
    return 'No sync sections enabled'
  }
  return `Sync sections: ${enabled.join(', ')}`
}
