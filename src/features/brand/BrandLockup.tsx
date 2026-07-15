interface BrandLockupProps {
  compact?: boolean
  inverse?: boolean
  showWordmark?: boolean
}

export function BrandLockup({ compact = false, inverse = false, showWordmark = true }: BrandLockupProps) {
  return (
    <div className={compact ? 'brand-lockup brand-lockup-compact' : 'brand-lockup'} aria-label="NinjaCrawler">
      <svg className="brand-mark" viewBox="0 0 256 256" aria-hidden="true" focusable="false">
        <path d="M56 196V60L200 196V60" />
      </svg>
      {showWordmark ? (
        <span className={inverse ? 'brand-wordmark brand-wordmark-inverse' : 'brand-wordmark'}>NinjaCrawler</span>
      ) : null}
    </div>
  )
}
