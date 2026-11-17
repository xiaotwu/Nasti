import type { PlaylistColumn, TrackItem } from "../../../contracts/ipc-commands";

export type Language = "zh-CN" | "en";
type Copy = readonly [zh: string, en: string];

const preferenceKey = "nasti.language";
let current: Language = "zh-CN";
try {
  if (localStorage.getItem(preferenceKey) === "en") current = "en";
} catch {
  // The browser preview may disable persistent storage.
}

export function language(): Language { return current; }
export function dateLocale(): string { return current === "en" ? "en-US" : "zh-CN"; }
export function t(zh: string, en: string): string { return current === "en" ? en : zh; }

export function selectLanguage(value: string): boolean {
  if ((value !== "zh-CN" && value !== "en") || value === current) return false;
  current = value;
  try {
    localStorage.setItem(preferenceKey, value);
  } catch {
    // Keep the selection for the lifetime of this window.
  }
  return true;
}

const sources: Record<string, { title: Copy; subtitle: Copy }> = {
  "acoustic-archive": { title: ["声学档案", "Acoustic archive"], subtitle: ["授权录音", "Licensed recordings"] },
  strings: { title: ["弦乐", "Strings"], subtitle: ["独奏大提琴", "Solo cello"] },
  nightwatch: { title: ["夜航", "Nightwatch"], subtitle: ["钢琴与大提琴", "Piano and cello"] },
  vocals: { title: ["声乐", "Vocals"], subtitle: ["传统歌曲", "Traditional songs"] },
  recital: { title: ["独奏", "Recital"], subtitle: ["古典录音", "Classical recordings"] },
  orbital: { title: ["轨道晨曦", "Orbital dawn"], subtitle: ["带伴奏画面", "With accompanying video"] },
};

const recordings: Record<string, { title: Copy; album: Copy }> = {
  "auld-lang-syne": { title: ["友谊地久天长", "Auld Lang Syne"], album: ["授权声乐录音", "Licensed vocal recording"] },
  "bach-cello-prelude": { title: ["无伴奏大提琴组曲第一号 · 前奏曲", "Cello Suite No. 1 · Prelude"], album: ["BWV 1007 · G 大调", "BWV 1007 · G major"] },
  "chopin-nocturne": { title: ["降 E 大调夜曲 · 作品 9 第 2 首", "Nocturne in E-flat major · Op. 9 No. 2"], album: ["肖邦夜曲", "Chopin nocturnes"] },
};

const composerNames: Record<string, Copy> = {
  "Johann Sebastian Bach": ["约翰·塞巴斯蒂安·巴赫", "Johann Sebastian Bach"],
  "Frédéric Chopin": ["弗雷德里克·肖邦", "Frédéric Chopin"],
};

function sourceLabels(id: string) {
  const source = id.replace(/-[1-9]$/, "");
  return Object.hasOwn(sources, source) ? sources[source] : undefined;
}

export function localizePlaylistTitle(playlist: Pick<PlaylistColumn, "playlistId" | "title">): string {
  const label = sourceLabels(playlist.playlistId);
  return label ? t(...label.title) : playlist.title;
}

export function localizeTrackTitle(track: Pick<TrackItem, "videoId" | "title">): string {
  const label = Object.hasOwn(recordings, track.videoId) ? recordings[track.videoId] : undefined;
  return label ? t(...label.title) : track.title;
}

export function localizeTrack(track: TrackItem): TrackItem {
  const label = Object.hasOwn(recordings, track.videoId) ? recordings[track.videoId] : undefined;
  if (!label) return track;
  return {
    ...track,
    title: t(...label.title),
    album: track.album ? { ...track.album, name: t(...label.album) } : track.album,
    artists: track.artists.map(artist => {
      const name = Object.values(composerNames).find(pair => pair.includes(artist.name));
      return name ? { ...artist, name: t(...name) } : artist;
    }),
  };
}

export function localizePlaylist(playlist: PlaylistColumn): PlaylistColumn {
  const labels = sourceLabels(playlist.playlistId);
  return {
    ...playlist,
    title: labels ? t(...labels.title) : playlist.title,
    subtitle: labels ? t(...labels.subtitle) : playlist.subtitle,
    tracks: playlist.tracks.map(localizeTrack),
  };
}

export function matchesLocalizedCatalog(query: string): boolean {
  if (!query) return false;
  const needle = query.toLowerCase();
  const titles = [...Object.values(sources).flatMap(({ title, subtitle }) => [title, subtitle]),
    ...Object.values(recordings).flatMap(({ title, album }) => [title, album]), ...Object.values(composerNames)];
  return titles.some(pair => t(...pair).toLowerCase().includes(needle));
}

export { localizeError } from "./localized-errors";
