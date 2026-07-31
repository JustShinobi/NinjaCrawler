import { describe, expect, it } from 'vitest'

import { resolveQueueJobPlan } from './queueJobPlan'

describe('resolveQueueJobPlan', () => {
  it('mostra a config efetiva do perfil para um sync disparado pelo grid', () => {
    const plan = resolveQueueJobPlan({
      provider: 'instagram',
      trigger: 'manual',
      profileSyncOptions: {
        instagram: {
          timeline: true,
          reels: true,
          stories: false,
          storiesUser: false,
          tagged: false,
        },
      },
    })

    expect(plan.origin).toBe('grid')
    expect(plan.scope).toBe('profile')
    expect(plan.sections.filter((chip) => chip.enabled).map((chip) => chip.code)).toEqual([
      'TL',
      'RE',
    ])
    expect(plan.summary).toContain('Sections: Timeline, Reels')
  })

  it('deixa o override vencer o perfil campo a campo', () => {
    const plan = resolveQueueJobPlan({
      provider: 'instagram',
      trigger: 'manual',
      profileSyncOptions: {
        instagram: {
          timeline: true,
          reels: false,
          stories: false,
          storiesUser: false,
          tagged: false,
        },
      },
      syncOptionsOverride: { instagram: { reels: true, timeline: false } },
    })

    expect(plan.sections.filter((chip) => chip.enabled).map((chip) => chip.code)).toEqual(['RE'])
  })

  it('trata um story do Companion como alvo pontual, sem trilha de sections', () => {
    const plan = resolveQueueJobPlan({
      provider: 'instagram',
      trigger: 'companion',
      profileSyncOptions: {
        instagram: {
          timeline: true,
          reels: true,
          stories: true,
          storiesUser: false,
          tagged: false,
        },
      },
      syncOptionsOverride: { instagram: { targetStoryMediaId: '3612345678901234567' } },
    })

    expect(plan.origin).toBe('companion')
    expect(plan.scope).toBe('single_story')
    expect(plan.sections).toEqual([])
    expect(plan.targetLabel).toBe('3612345678901234567')
    expect(plan.summary).toContain('1 story')
  })

  it('encurta a URL de um vídeo pontual do TikTok', () => {
    const plan = resolveQueueJobPlan({
      provider: 'tiktok',
      trigger: 'companion',
      syncOptionsOverride: {
        tiktok: { targetVideoUrl: 'https://www.tiktok.com/@dance.hub/video/7351234567890?lang=en' },
      },
    })

    expect(plan.scope).toBe('single_video')
    expect(plan.targetLabel).toBe('7351234567890')
  })

  it('lista os modificadores do job como notes', () => {
    const plan = resolveQueueJobPlan({
      provider: 'instagram',
      trigger: 'manual_force_imported_backfill',
      runMode: 'force_imported_backfill',
      profileSyncOptions: {
        instagram: {
          timeline: true,
          reels: false,
          stories: false,
          storiesUser: false,
          tagged: false,
        },
      },
      syncOptionsOverride: { instagram: { fullScan: true } },
    })

    expect(plan.notes).toContain('Full scan')
    expect(plan.notes).toContain('force imported backfill')
  })

  it('não inventa trilha para providers sem sections definidas', () => {
    const plan = resolveQueueJobPlan({ provider: 'youtube', trigger: 'scheduler' })

    expect(plan.origin).toBe('scheduler')
    expect(plan.sections).toEqual([])
    expect(plan.summary).toBe('Origin: Scheduler')
  })
})
