import { describe, expect, it } from 'vitest'

import {
  groupRecentTasks,
  stripSyncSummaryPrefix,
  type RecentGroupableTask,
} from './queueRecentGroups'

function task(overrides: Partial<RecentGroupableTask> & { key: string }): RecentGroupableTask {
  return {
    sourceId: 'source-a',
    operation: 'Sync',
    status: 'succeeded',
    finishedAt: '2026-03-11T12:00:00Z',
    ...overrides,
  }
}

describe('groupRecentTasks', () => {
  it('funde a geração de thumbnails na entrada do sync que a disparou', () => {
    const groups = groupRecentTasks([
      task({ key: 'thumb', operation: 'Thumbnail', finishedAt: '2026-03-11T12:00:30Z' }),
      task({ key: 'sync', finishedAt: '2026-03-11T12:00:00Z' }),
    ])

    expect(groups).toHaveLength(1)
    expect(groups[0]!.primary.key).toBe('sync')
    expect(groups[0]!.thumbnail?.key).toBe('thumb')
    // A entrada é ordenada pelo fim da última etapa.
    expect(groups[0]!.finishedAt).toBe('2026-03-11T12:00:30Z')
  })

  it('eleva o pior status das etapas para a entrada', () => {
    const groups = groupRecentTasks([
      task({
        key: 'thumb',
        operation: 'Thumbnail',
        status: 'warning',
        finishedAt: '2026-03-11T12:00:30Z',
      }),
      task({ key: 'sync', status: 'succeeded' }),
    ])

    expect(groups[0]!.status).toBe('warning')
  })

  it('mantém uma thumbnail avulsa como entrada própria', () => {
    const groups = groupRecentTasks([
      task({ key: 'thumb', operation: 'Thumbnail', finishedAt: '2026-03-11T12:00:00Z' }),
    ])

    expect(groups).toHaveLength(1)
    expect(groups[0]!.primary.key).toBe('thumb')
    expect(groups[0]!.thumbnail).toBeUndefined()
  })

  it('não pareia thumbnail com sync de outro perfil', () => {
    const groups = groupRecentTasks([
      task({
        key: 'thumb',
        sourceId: 'source-b',
        operation: 'Thumbnail',
        finishedAt: '2026-03-11T12:00:30Z',
      }),
      task({ key: 'sync', sourceId: 'source-a' }),
    ])

    expect(groups).toHaveLength(2)
  })

  it('não pareia com um sync fora da janela de tempo', () => {
    const groups = groupRecentTasks([
      task({ key: 'thumb', operation: 'Thumbnail', finishedAt: '2026-03-11T13:00:00Z' }),
      task({ key: 'sync', finishedAt: '2026-03-11T12:00:00Z' }),
    ])

    expect(groups).toHaveLength(2)
  })

  it('não pareia com um sync que terminou depois da thumbnail', () => {
    const groups = groupRecentTasks([
      task({ key: 'sync', finishedAt: '2026-03-11T12:01:00Z' }),
      task({ key: 'thumb', operation: 'Thumbnail', finishedAt: '2026-03-11T12:00:00Z' }),
    ])

    expect(groups).toHaveLength(2)
  })

  it('dá a cada thumbnail um sync distinto quando o perfil roda duas vezes', () => {
    const groups = groupRecentTasks([
      task({ key: 'thumb-2', operation: 'Thumbnail', finishedAt: '2026-03-11T12:02:30Z' }),
      task({ key: 'sync-2', finishedAt: '2026-03-11T12:02:00Z' }),
      task({ key: 'thumb-1', operation: 'Thumbnail', finishedAt: '2026-03-11T12:00:30Z' }),
      task({ key: 'sync-1', finishedAt: '2026-03-11T12:00:00Z' }),
    ])

    expect(groups).toHaveLength(2)
    expect(groups.map((group) => group.primary.key)).toEqual(['sync-2', 'sync-1'])
    expect(groups[0]!.thumbnail?.key).toBe('thumb-2')
    expect(groups[1]!.thumbnail?.key).toBe('thumb-1')
  })

  it('deixa delete, migration e single video intactos', () => {
    const groups = groupRecentTasks([
      task({ key: 'delete', operation: 'Delete' }),
      task({ key: 'migration', operation: 'Migration' }),
      task({ key: 'single', operation: 'Single' }),
    ])

    expect(groups).toHaveLength(3)
    expect(groups.every((group) => group.thumbnail === undefined)).toBe(true)
  })
})

describe('stripSyncSummaryPrefix', () => {
  it('remove o prefixo redundante de provider e status', () => {
    expect(stripSyncSummaryPrefix('Instagram sync succeeded. Downloaded 2 media items.')).toBe(
      'Downloaded 2 media items.',
    )
    expect(stripSyncSummaryPrefix('X / Twitter sync succeeded. No new media downloaded.')).toBe(
      'No new media downloaded.',
    )
  })

  it('preserva o resumo quando ele é só o prefixo', () => {
    expect(stripSyncSummaryPrefix('Instagram sync succeeded.')).toBe('Instagram sync succeeded.')
  })

  it('não mexe em resumos de outras operações', () => {
    expect(stripSyncSummaryPrefix('Generated 2, kept 195 existing.')).toBe(
      'Generated 2, kept 195 existing.',
    )
  })
})
