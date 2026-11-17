// Browser preview uses the same authorized catalog and a real browser playback adapter.
import type {
  AuthorizedCatalog, NastiIpcContract, PlaybackController, PlaylistColumn, PlaylistImportPreview, TrackItem,
} from "./ipc-commands";

const libraryKey = "nasti-authorized-library-v1";
const cacheName = "nasti-authorized-vault-v1";
const defaultLimit = 256 * 1024 ** 2;
const maxLimit = 2 * 1024 ** 3;
const columnCount = 9;

type Sources = AuthorizedCatalog["playlists"][number];
type Entry = { entryId: string; videoId: string; sourceIndex?: number };
type Collection = {
  id: string;
  sourceId: string;
  columnIndex: number;
  entries: Entry[];
};
type SavedMedia = { url: string; bytes: number };
type LibraryState = {
  columns: Collection[];
  limit: number;
  nextEntry: number;
  vault: Record<string, SavedMedia[]>;
  sourceIndexMigrated?: boolean;
};

export function createMockIpc(catalog: AuthorizedCatalog, playback: PlaybackController): NastiIpcContract {
  const tracks = new Map(catalog.tracks.map(track => [track.videoId, track]));
  const sources = new Map(catalog.playlists.map(playlist => [playlist.id, playlist]));
  const state: LibraryState = { columns: [], limit: defaultLimit, nextEntry: 1, vault: {} };
  const makeColumn = (playlist: Sources, index: number): Collection => ({
    id: `${playlist.id}-${index + 1}`,
    sourceId: playlist.id,
    columnIndex: index,
    entries: playlist.videoIds.map((videoId, sourceIndex) => ({ videoId, sourceIndex, entryId: String(state.nextEntry++) })),
  });
  try {
    const saved = localStorage.getItem(libraryKey);
    if (saved) Object.assign(state, JSON.parse(saved));
  } catch {
    localStorage.removeItem(libraryKey);
  }
  if (!state.columns.length) state.columns = catalog.playlists.slice(0, 5).map(makeColumn);
  const persist = () => localStorage.setItem(libraryKey, JSON.stringify(state));
  if (!state.sourceIndexMigrated) {
    for (const column of state.columns) {
      const source = sources.get(column.sourceId);
      let from = 0;
      for (const entry of column.entries) {
        const index = source?.videoIds.indexOf(entry.videoId, from) ?? -1;
        if (index < 0) continue;
        entry.sourceIndex = index;
        from = index + 1;
      }
    }
    state.sourceIndexMigrated = true;
  }
  const getTrack = (videoId: string): TrackItem => {
    const track = tracks.get(videoId);
    if (!track) throw new Error("授权曲库中没有此曲目");
    const saved = state.vault[videoId] ?? [];
    return { ...track, isVaultDownloaded: saved.some(item => item.url === track.audioUrl),
      isVaultVideoDownloaded: saved.some(item => item.url === track.mvUrl) };
  };
  const columnView = (column: Collection, isEditable = true): PlaylistColumn => {
    const source = sources.get(column.sourceId);
    if (!source) throw new Error("授权曲库中没有此歌单");
    return {
      playlistId: column.id,
      columnIndex: column.columnIndex,
      codeName: `RL-${String(column.columnIndex + 1).padStart(2, "0")}`,
      title: source.title,
      subtitle: source.subtitle,
      isEditable,
      tracks: column.entries.map((entry, index) => ({
        ...getTrack(entry.videoId), entryId: entry.entryId, trackNumber: index + 1,
      })),
    };
  };
  const usage = () => Object.values(state.vault).flat().reduce((total, item) => total + item.bytes, 0);
  const sourceFor = (reference: string) => {
    const source = sources.get(reference.trim().replace(/^nasti:\/\/playlist\//, ""));
    if (!source) throw new Error("请输入目录中的歌单 ID 或 nasti://playlist/ 链接");
    return source;
  };
  const firstColumn = (source: Sources) => state.columns.filter(column => column.sourceId === source.id)
    .sort((a, b) => a.columnIndex - b.columnIndex)[0];
  const preview = (source: Sources): PlaylistImportPreview => {
    const column = firstColumn(source);
    return { sourceId: source.id, title: source.title, tracks: source.videoIds.map((videoId, sourceIndex) => ({
      sourceIndex, track: getTrack(videoId), imported: Boolean(column?.entries.some(entry => entry.sourceIndex === sourceIndex && entry.videoId === videoId)),
    })) };
  };

  // Keep the bytes and local index in sync when two controls are used in quick succession.
  let vaultWrite = Promise.resolve();
  const writeVault = <T>(operation: () => Promise<T>): Promise<T> => {
    const result = vaultWrite.then(operation);
    vaultWrite = result.then(() => undefined, () => undefined);
    return result;
  };

  return {
    async catalog_search_archive() { throw new Error("请在 Nasti 桌面版使用在线曲库"); },
    playback_load: playback.playback_load.bind(playback),
    playback_get_state: playback.playback_get_state.bind(playback),
    playback_set_playing: playback.playback_set_playing.bind(playback),
    playback_seek: playback.playback_seek.bind(playback),
    playback_set_volume: playback.playback_set_volume.bind(playback),
    playback_list_outputs: async () => ({ supported: false, selectedDeviceId: null, defaultDeviceId: null, devices: [] }),
    playback_set_output: async () => { throw new Error("当前平台不支持选择播放设备"); },
    playback_set_video_frame: playback.playback_set_video_frame.bind(playback),
    playback_prepare: playback.playback_prepare.bind(playback),
    playback_commit: playback.playback_commit.bind(playback),
    playback_cancel_preparation: playback.playback_cancel_preparation.bind(playback),
    async catalog_fetch_columns() {
      return state.columns.slice().sort((a, b) => a.columnIndex - b.columnIndex).map(column => columnView(column));
    },
    async catalog_import_playlist_url({ urlOrId, targetColumn }) {
      const source = sourceFor(urlOrId);
      const occupied = state.columns.map(column => column.columnIndex);
      const index = targetColumn ?? Array.from({ length: columnCount }, (_, i) => i).find(i => !occupied.includes(i));
      if (index === undefined) throw new Error("九列均已使用");
      if (!Number.isInteger(index) || index < 0 || index >= columnCount) throw new Error("请选择第 1 至第 9 列");
      if (occupied.includes(index)) throw new Error("该列已有歌单");
      const column = makeColumn(source, index);
      state.columns.push(column);
      persist();
      return columnView(column);
    },
    async catalog_preview_playlist({ urlOrId }) {
      return preview(sourceFor(urlOrId));
    },
    async catalog_apply_playlist_import({ sourceId, sourceIndex }) {
      const source = sourceFor(sourceId);
      if (sourceIndex !== undefined && (!Number.isSafeInteger(sourceIndex) || !source.videoIds[sourceIndex])) throw new Error("源曲目已变化");
      let column = firstColumn(source);
      if (!column) {
        const position = Array.from({ length: columnCount }, (_, i) => i).find(i => !state.columns.some(column => column.columnIndex === i));
        if (position === undefined) throw new Error("九列均已使用");
        column = { id: source.id + "-" + (position + 1), sourceId, columnIndex: position, entries: [] };
        state.columns.push(column);
      }
      for (const [i, videoId] of source.videoIds.entries()) {
        if (sourceIndex !== undefined && i !== sourceIndex) continue;
        if (column.entries.some(entry => entry.sourceIndex === i)) continue;
        column.entries.push({ entryId: String(state.nextEntry++), videoId, sourceIndex: i });
      }
      if (sourceIndex === undefined) column.entries.sort((a, b) => (a.sourceIndex ?? Infinity) - (b.sourceIndex ?? Infinity));
      persist();
      return preview(source);
    },
    async catalog_search({ query, filter }) {
      const term = query.trim().toLowerCase();
      const matches = (value: string | null | undefined) => value?.toLowerCase().includes(term) ?? false;
      const found = catalog.tracks.filter(track =>
        matches(track.title) || matches(track.album?.name) || track.artists.some(artist => matches(artist.name)),
      );
      return {
        songs: filter === "all" || filter === "songs" ? found.map(track => getTrack(track.videoId)) : [],
        videos: filter === "all" || filter === "videos" ? found.filter(track => track.mvUrl).map(track => getTrack(track.videoId)) : [],
        playlists: filter === "all" || filter === "playlists" ? catalog.playlists.flatMap((source, index) =>
          matches(source.title) || matches(source.subtitle) || matches(source.id)
            ? [columnView({
              id: source.id, sourceId: source.id, columnIndex: index % columnCount,
              entries: source.videoIds.map((videoId, i) => ({ videoId, entryId: `source-${source.id}-${i}` })),
            }, false)]
            : [],
        ) : [],
      };
    },
    async playlist_add_track({ playlistId, videoId }) {
      getTrack(videoId);
      const column = state.columns.find(item => item.id === playlistId);
      if (!column) throw new Error("没有此歌单");
      const entryId = String(state.nextEntry++);
      column.entries.push({ entryId, videoId });
      persist();
      return { entryId };
    },
    async playlist_remove_track({ playlistId, entryId }) {
      const column = state.columns.find(item => item.id === playlistId);
      const index = column?.entries.findIndex(entry => entry.entryId === entryId) ?? -1;
      if (!column || index < 0) return false;
      column.entries.splice(index, 1);
      persist();
      return true;
    },
    async playlist_remove_entries({ playlistId, entryIds }) {
      const column = state.columns.find(item => item.id === playlistId);
      if (!column) throw new Error("没有此歌单");
      const requested = new Set(entryIds);
      const entries = column.entries.flatMap((entry, index, all) => requested.has(entry.entryId) ? [{
        entryId: entry.entryId, videoId: entry.videoId, sourceIndex: entry.sourceIndex ?? null, index,
        previousId: all[index - 1]?.entryId ?? null, nextId: all[index + 1]?.entryId ?? null,
      }] : []);
      if (!entries.length || entries.length !== requested.size) throw new Error("曲目入列记录已变化，请重新选择");
      column.entries = column.entries.filter(entry => !requested.has(entry.entryId));
      persist();
      return { playlistId, entries };
    },
    async playlist_restore_entries({ removal }) {
      const column = state.columns.find(item => item.id === removal.playlistId);
      if (!column) throw new Error("没有此歌单");
      const entries = [...column.entries];
      for (const entry of removal.entries.slice().sort((a, b) => a.index - b.index)) {
        if (entries.some(item => item.entryId === entry.entryId || (entry.sourceIndex !== null && item.sourceIndex === entry.sourceIndex))) continue;
        getTrack(entry.videoId);
        const previous = entries.findIndex(item => item.entryId === entry.previousId);
        const next = entries.findIndex(item => item.entryId === entry.nextId);
        const index = previous >= 0 ? previous + 1 : next >= 0 ? next : Math.min(entries.length, entry.index);
        entries.splice(index, 0, { entryId: entry.entryId, videoId: entry.videoId, sourceIndex: entry.sourceIndex ?? undefined });
      }
      column.entries = entries;
      persist();
    },
    vault_download_track({ videoId, downloadMv }) {
      return writeVault(async () => {
        const track = getTrack(videoId);
        if (!track.audioUrl || (downloadMv && !track.mvUrl)) throw new Error("曲库中没有此媒体版本");
        const saved = state.vault[videoId] ?? [];
        const urls = [track.audioUrl, ...(downloadMv && track.mvUrl ? [track.mvUrl] : [])]
          .filter(url => !saved.some(item => item.url === url));
        const cache = await caches.open(cacheName);
        const added: SavedMedia[] = [];
        let total = usage();
        try {
          for (const url of urls) {
            const response = await fetch(url);
            if (!response.ok) throw new Error("无法读取授权媒体");
            const body = await response.blob();
            total += body.size;
            if (total > state.limit) throw new Error("Vault 已达存储上限");
            await cache.put(url, new Response(body, { headers: { "Content-Type": body.type } }));
            added.push({ url, bytes: body.size });
          }
          state.vault[videoId] = [...saved, ...added];
          persist();
        } catch (error) {
          await Promise.all(added.map(item => cache.delete(item.url)));
          if (saved.length) state.vault[videoId] = saved;
          else delete state.vault[videoId];
          throw error;
        }
        return getTrack(videoId);
      });
    },
    vault_remove_track({ videoId }) {
      return writeVault(async () => {
        getTrack(videoId);
        const saved = state.vault[videoId];
        if (!saved) return false;
        const cache = await caches.open(cacheName);
        await Promise.all(saved.map(item => cache.delete(item.url)));
        delete state.vault[videoId];
        persist();
        return true;
      });
    },
    async vault_get_storage_stats() {
      await vaultWrite;
      return { usedBytes: usage(), limitBytes: state.limit, trackCount: Object.keys(state.vault).length };
    },
    async vault_list_tracks() {
      await vaultWrite;
      return Object.entries(state.vault).map(([id, items]) => ({
        track: getTrack(id), bytes: items.reduce((total, item) => total + item.bytes, 0),
      }));
    },
    vault_set_storage_limit({ limitBytes }) {
      return writeVault(async () => {
        if (!Number.isSafeInteger(limitBytes) || limitBytes < usage() || limitBytes > maxLimit) {
          throw new Error("上限必须介于当前用量与 2 GiB 之间");
        }
        state.limit = limitBytes;
        persist();
      });
    },
    async lyrics_get_synced({ videoId }) {
      getTrack(videoId);
      return catalog.lyrics[videoId] ?? [];
    },
  };
}
