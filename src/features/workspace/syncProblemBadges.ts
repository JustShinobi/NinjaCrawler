export function syncProblemBadgeLabel(syncProblemCode?: string): string {
  const code = (syncProblemCode ?? '').trim().toLowerCase()
  if (!code) {
    return 'Sync issue'
  }

  switch (code) {
    case 'instagram_profile_private_or_restricted':
    case 'tiktok_profile_private_or_restricted':
      return 'Private profile'
    case 'instagram_username_unresolvable':
    case 'tiktok_profile_unavailable':
      return 'Profile unavailable'
    case 'auth_required':
      return 'Auth required'
    // The handle now belongs to a different provider account. Downloads are
    // paused so the new owner's media is not mixed into this archive.
    case 'handle_recycled':
      return 'Handle taken over'
    default:
      return 'Sync issue'
  }
}

export function isBlockingSyncProblem(syncProblemCode?: string): boolean {
  const code = (syncProblemCode ?? '').trim().toLowerCase()
  if (!code) {
    return false
  }
  // Any known sync problem — private/restricted, unavailable, auth required —
  // pauses the source (`ready_for_download` is turned off), so every non-empty
  // problem code is blocking.
  return true
}
