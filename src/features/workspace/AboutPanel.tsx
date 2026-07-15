import { useState } from 'react'
import type { AppBuildInfo, AppUpdateStatus } from '../../domain/models'
import { BrandLockup } from '../brand/BrandLockup'

interface AboutPath {
  label: string
  value: string
}

interface AboutPanelProps {
  buildInfo?: AppBuildInfo
  updateStatus?: AppUpdateStatus
  updateChecking: boolean
  updateError?: string
  workspaceRoot: string
  databasePath: string
  mediaRoot: string
  profileCount: number
  accountCount: number
  planCount: number
  onCheckUpdate: () => void
  onOpenRelease: (url: string) => void
}

export function AboutPanel({
  accountCount,
  buildInfo,
  databasePath,
  mediaRoot,
  onCheckUpdate,
  onOpenRelease,
  planCount,
  profileCount,
  updateChecking,
  updateError,
  updateStatus,
  workspaceRoot,
}: AboutPanelProps) {
  const [copiedPath, setCopiedPath] = useState<string>()
  const [copyError, setCopyError] = useState<string>()
  const paths: AboutPath[] = [
    { label: 'Workspace', value: workspaceRoot },
    { label: 'Database', value: databasePath },
    { label: 'Media root', value: mediaRoot },
  ]

  async function copyPath(path: AboutPath) {
    setCopyError(undefined)
    try {
      await navigator.clipboard.writeText(path.value)
      setCopiedPath(path.label)
    } catch (error) {
      setCopiedPath(undefined)
      setCopyError(error instanceof Error ? error.message : 'Clipboard access is unavailable.')
    }
  }

  const releaseMessage = updateChecking
    ? 'Checking GitHub…'
    : updateError
      ? updateError
      : updateStatus
        ? `v${updateStatus.latestVersion}${updateStatus.updateAvailable ? ' is available.' : ' is the latest release.'}`
        : 'Not checked yet.'

  return (
    <section className="about-layout">
      <header className="about-brand-hero">
        <BrandLockup />
        <div className="about-build-identity">
          <span className="status status-ready">{buildInfo?.channel ?? 'Application'}</span>
          <strong>{buildInfo?.displayVersion ?? 'Loading build information…'}</strong>
        </div>
      </header>

      <article className="panel about-update-panel">
        <div className="about-update-copy">
          <p className="eyebrow">Application update</p>
          <h3>{updateStatus?.updateAvailable ? 'A newer release is ready' : 'NinjaCrawler is up to date'}</h3>
          <p className={updateError ? 'about-inline-error' : 'muted-copy'} role={updateError ? 'alert' : undefined}>
            {releaseMessage}
          </p>
          {buildInfo?.channel === 'development' ? (
            <p className="muted-copy">Development builds are identified by commit and are not compared by age.</p>
          ) : null}
        </div>
        <div className="about-update-actions">
          <button className="ghost-button" disabled={updateChecking} onClick={onCheckUpdate} type="button">
            {updateChecking ? 'Checking…' : 'Check again'}
          </button>
          {updateStatus?.releaseUrl ? (
            <button
              className={updateStatus.updateAvailable ? 'primary-button' : 'ghost-button'}
              onClick={() => onOpenRelease(updateStatus.releaseUrl)}
              type="button"
            >
              View / Download v{updateStatus.latestVersion} on GitHub
            </button>
          ) : null}
        </div>
      </article>

      <div className="about-info-grid">
        <article className="panel about-info-panel">
          <div className="about-section-heading">
            <div>
              <p className="eyebrow">Workspace</p>
              <h3>Local paths</h3>
            </div>
          </div>
          <dl className="about-path-list">
            {paths.map((path) => (
              <div className="about-path-row" key={path.label}>
                <div className="about-path-copy">
                  <dt>{path.label}</dt>
                  <dd title={path.value}>{path.value}</dd>
                </div>
                <button className="ghost-button about-copy-button" onClick={() => void copyPath(path)} type="button">
                  {copiedPath === path.label ? 'Copied' : 'Copy'}
                </button>
              </div>
            ))}
          </dl>
          {copyError ? <p className="about-inline-error" role="alert">{copyError}</p> : null}
        </article>

        <article className="panel about-info-panel">
          <div className="about-section-heading">
            <div>
              <p className="eyebrow">Runtime</p>
              <h3>Environment</h3>
            </div>
          </div>
          <dl className="about-metric-list">
            <div><dt>Profiles</dt><dd>{profileCount} registered</dd></div>
            <div><dt>Accounts</dt><dd>{accountCount} configured</dd></div>
            <div><dt>Plans</dt><dd>{planCount} scheduled</dd></div>
          </dl>
        </article>
      </div>
    </section>
  )
}
