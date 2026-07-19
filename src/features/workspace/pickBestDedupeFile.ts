import type { MediaDedupeFile } from "../../domain/models";

/**
 * Ranks a file for "best quality in group" purposes, VDF-style: higher
 * resolution wins first, then higher bitrate, then larger size, then the most
 * recently modified file as a final tiebreaker. A file missing a given metric
 * is treated as the lowest possible value for that metric so missing data
 * never accidentally wins a comparison.
 */
function bestFileRank(
  file: MediaDedupeFile,
): readonly [number, number, number, number] {
  const resolution =
    file.width && file.height ? file.width * file.height : -1;
  const bitrate =
    file.bitrateKbps && file.bitrateKbps > 0 ? file.bitrateKbps : -1;
  const size = file.sizeBytes > 0 ? file.sizeBytes : -1;
  const modifiedAt = file.modifiedAt ?? -1;
  return [resolution, bitrate, size, modifiedAt];
}

/**
 * Picks the best-quality file within a duplicate group: resolution, then
 * bitrate, then size, then most-recently-modified as tiebreakers. Exported so
 * the ranking rule can be unit-tested independently of the UI.
 */
export function pickBestDedupeFile(
  files: readonly MediaDedupeFile[],
): MediaDedupeFile | undefined {
  if (files.length === 0) return undefined;
  return files.reduce((best, file) => {
    const candidate = bestFileRank(file);
    const current = bestFileRank(best);
    for (let index = 0; index < candidate.length; index += 1) {
      if (candidate[index] !== current[index]) {
        return candidate[index] > current[index] ? file : best;
      }
    }
    return best;
  });
}
