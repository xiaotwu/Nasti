import type { PlaylistColumn, TrackItem } from "../../../contracts/ipc-commands";

export type RepeatMode = "off" | "all" | "one";

export function repeatMode(preference: string | null): RepeatMode {
  return preference === "all" || preference === "one" ? preference : "off";
}

export function nextRepeatMode(current: RepeatMode): RepeatMode {
  if (current === "off") return "all";
  return current === "all" ? "one" : "off";
}

export function occurrenceId(track: TrackItem, index: number): string {
  return track.entryId ?? track.videoId + ":" + index;
}

function adjacentIndex(index: number, length: number, direction: 1 | -1, wrap: boolean): number | null {
  const next = index + direction;
  if (wrap) return (next + length) % length;
  return next >= 0 && next < length ? next : null;
}

export class TrackNavigation {
  private readonly random: () => number;
  private shuffleOrder: { playlistId: string; entries: string[] } | null = null;

  constructor(random: () => number = Math.random) {
    this.random = random;
  }

  reset(): void {
    this.shuffleOrder = null;
  }

  step(column: PlaylistColumn, currentIndex: number, direction: 1 | -1, options: { shuffle: boolean; wrap: boolean }): number | null {
    const length = column.tracks.length;
    if (!column.tracks[currentIndex]) return null;
    if (!options.shuffle) return adjacentIndex(currentIndex, length, direction, options.wrap);

    const entries = column.tracks.map(occurrenceId);
    const byId = new Map(entries.map((id, index) => [id, index]));
    let order = this.shuffleOrder?.playlistId === column.playlistId ? this.shuffleOrder.entries : null;
    // Catalog refresh can reorder source positions without changing the membership of the shuffled queue.
    if (!order || order.length !== length || !order.every(entry => byId.has(entry))) {
      const current = entries[currentIndex];
      order = entries.filter(entry => entry !== current);
      for (let i = order.length - 1; i > 0; i--) {
        const peer = Math.floor(this.random() * (i + 1));
        [order[i], order[peer]] = [order[peer], order[i]];
      }
      order.unshift(current);
      this.shuffleOrder = { playlistId: column.playlistId, entries: order };
    }
    const next = adjacentIndex(order.indexOf(entries[currentIndex]), length, direction, options.wrap);
    return next === null ? null : byId.get(order[next]) ?? null;
  }
}
