import { describe, expect, it } from 'vitest'
import type { ConnectorRuntimeStatus } from '../../domain/models'
import { connectorsNeedPreparation } from './connectorPreparation'

function runtime(overrides: Partial<ConnectorRuntimeStatus> = {}): ConnectorRuntimeStatus {
  return {
    key: 'gallery-dl',
    displayName: 'gallery-dl',
    managementMode: 'managed',
    bundledVersion: '1.31.9',
    updateAvailable: false,
    status: 'up_to_date',
    activeVersion: '1.31.9',
    activePath: 'C:\\Users\\test\\AppData\\Local\\NinjaCrawler\\connectors\\gallery-dl\\1.31.9\\gallery-dl.exe',
    ...overrides,
  }
}

describe('connectorsNeedPreparation', () => {
  it('blocks the workspace when a connector is not installed', () => {
    expect(connectorsNeedPreparation([
      runtime(),
      runtime({ key: 'yt-dlp', activeVersion: undefined, activePath: undefined, status: 'not_installed' }),
    ])).toBe(true)
  })

  it('accepts valid managed and custom connector runtimes', () => {
    expect(connectorsNeedPreparation([
      runtime(),
      runtime({ key: 'yt-dlp', managementMode: 'custom', activeVersion: '2026.03.03', activePath: 'D:\\Tools\\yt-dlp.exe' }),
    ])).toBe(false)
  })

  it('keeps an invalid custom override blocked', () => {
    expect(connectorsNeedPreparation([
      runtime({ managementMode: 'custom', activeVersion: undefined, status: 'error', lastError: 'Version probe failed.' }),
    ])).toBe(true)
  })
})
