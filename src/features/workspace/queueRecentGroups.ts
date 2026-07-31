export type QueueResultStatus = 'succeeded' | 'warning' | 'failed' | 'skipped'

export interface RecentGroupableTask {
  key: string
  sourceId: string
  operation: 'Sync' | 'Delete' | 'Single' | 'Migration' | 'Thumbnail'
  status: QueueResultStatus
  finishedAt: string
}

export interface QueueRecentGroup<T extends RecentGroupableTask> {
  key: string
  /** Operação que o usuário pediu — é ela que dá identidade à entrada. */
  primary: T
  /** Geração de thumbnails disparada por este sync, quando houve. */
  thumbnail?: T
  status: QueueResultStatus
  /** Fim da última etapa do grupo: é por ele que a lista é ordenada. */
  finishedAt: string
}

/**
 * Janela de pareamento. Todo sync bem-sucedido enfileira thumbnails para o mesmo
 * perfil logo em seguida, então o par nasce próximo no tempo; uma janela larga
 * demais roubaria o resultado de um sync manual seguinte do mesmo perfil.
 */
const DEFAULT_PAIRING_WINDOW_MS = 10 * 60 * 1000

const STATUS_SEVERITY: Record<QueueResultStatus, number> = {
  skipped: 0,
  succeeded: 1,
  warning: 2,
  failed: 3,
}

function worstStatus(left: QueueResultStatus, right: QueueResultStatus): QueueResultStatus {
  return STATUS_SEVERITY[right] > STATUS_SEVERITY[left] ? right : left
}

function timestamp(value: string): number {
  const parsed = Date.parse(value)
  return Number.isNaN(parsed) ? 0 : parsed
}

/**
 * Funde cada geração automática de thumbnails na entrada do sync que a disparou.
 *
 * O backend não correlaciona as duas: o sync termina, chama
 * `media_thumbnail_runtime::enqueue` e esquece. O pareamento aqui é por
 * `sourceId` + proximidade temporal, escolhendo o sync mais recente que ainda
 * não foi pareado e que terminou antes da thumbnail. Se o mesmo perfil for
 * sincronizado duas vezes dentro da janela, a contagem de thumbs pode cair na
 * entrada vizinha — erro cosmético, sem efeito sobre o que é acionável.
 *
 * Thumbnails sem sync correspondente (rodadas pela Maintenance) permanecem
 * como entrada própria.
 */
export function groupRecentTasks<T extends RecentGroupableTask>(
  tasks: T[],
  pairingWindowMs: number = DEFAULT_PAIRING_WINDOW_MS,
): QueueRecentGroup<T>[] {
  const syncCandidates = tasks
    .filter((task) => task.operation === 'Sync')
    .sort((left, right) => timestamp(left.finishedAt) - timestamp(right.finishedAt))

  const thumbnailBySyncKey = new Map<string, T>()
  const pairedThumbnailKeys = new Set<string>()
  const pairedSyncKeys = new Set<string>()

  for (const thumbnail of tasks) {
    if (thumbnail.operation !== 'Thumbnail') {
      continue
    }
    const thumbnailAt = timestamp(thumbnail.finishedAt)
    // O sync mais recente que ainda cabe na janela: percorrendo do mais antigo
    // para o mais novo, o último que satisfaz a condição é o melhor candidato.
    let match: T | undefined
    for (const sync of syncCandidates) {
      if (sync.sourceId !== thumbnail.sourceId || pairedSyncKeys.has(sync.key)) {
        continue
      }
      const syncAt = timestamp(sync.finishedAt)
      if (syncAt <= thumbnailAt && thumbnailAt - syncAt <= pairingWindowMs) {
        match = sync
      }
    }
    if (!match) {
      continue
    }
    thumbnailBySyncKey.set(match.key, thumbnail)
    pairedSyncKeys.add(match.key)
    pairedThumbnailKeys.add(thumbnail.key)
  }

  const groups: QueueRecentGroup<T>[] = []
  for (const task of tasks) {
    if (task.operation === 'Thumbnail' && pairedThumbnailKeys.has(task.key)) {
      continue
    }
    const thumbnail = thumbnailBySyncKey.get(task.key)
    groups.push({
      key: task.key,
      primary: task,
      thumbnail,
      status: thumbnail ? worstStatus(task.status, thumbnail.status) : task.status,
      finishedAt:
        thumbnail && timestamp(thumbnail.finishedAt) > timestamp(task.finishedAt)
          ? thumbnail.finishedAt
          : task.finishedAt,
    })
  }

  return groups.sort((left, right) => timestamp(right.finishedAt) - timestamp(left.finishedAt))
}

/**
 * Remove o prefixo redundante do resumo do sync ("Instagram sync succeeded.").
 * O provider e o status já estão na headline da entrada.
 */
export function stripSyncSummaryPrefix(summary: string): string {
  const stripped = summary.replace(
    /^[a-z0-9/\s.]*?sync\s+(succeeded|failed|completed|finished)\.?\s*/i,
    '',
  )
  const next = stripped.trim()
  return next.length > 0 ? next : summary
}
