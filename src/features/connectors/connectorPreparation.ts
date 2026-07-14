import type { ConnectorRuntimeStatus } from '../../domain/models'

export function connectorsNeedPreparation(runtimes: ConnectorRuntimeStatus[]): boolean {
  return runtimes.some((runtime) => !runtime.activeVersion)
}
