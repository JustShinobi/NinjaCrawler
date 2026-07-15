export function formatAuthState(state: string | undefined): string {
  if (!state) {
    return 'Draft'
  }
  if (state === 'ready') {
    return 'Ready'
  }
  return state.replace(/_/g, ' ').replace(/\b\w/g, (char) => char.toUpperCase())
}
