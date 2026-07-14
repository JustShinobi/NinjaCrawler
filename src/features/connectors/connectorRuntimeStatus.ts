import type { ConnectorRuntimeStatus } from '../../domain/models'

export function connectorRuntimeStatusLabel(runtime: ConnectorRuntimeStatus) {
  switch (runtime.status) {
    case 'not_installed':
      return 'Not installed'
    case 'checking':
      return 'Checking'
    case 'downloading':
      return 'Downloading'
    case 'pending_activation':
      return 'Pending activation'
    case 'custom_override':
      return 'Custom override'
    case 'error':
      return 'Error'
    case 'update_available':
      return 'Update available'
    default:
      return 'Up to date'
  }
}

export function connectorRuntimeStatusClassName(runtime: ConnectorRuntimeStatus) {
  switch (runtime.status) {
    case 'error':
      return 'status-failed'
    case 'update_available':
    case 'pending_activation':
      return 'status-degraded'
    default:
      return 'status-ready'
  }
}
