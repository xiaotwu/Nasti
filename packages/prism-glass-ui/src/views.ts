import type { ArchiveSearchResult, LyricLine, PlaylistColumn, TrackItem } from "../../../contracts/ipc-commands";
import { icon, type IconName } from "./icons";
import { dateLocale, t } from "./i18n";
import { durationLabel, element, safeAssetUrl } from "./layout";

export const trackArtists = (track: TrackItem | null): string =>
  track?.artists.map(artist => artist.name).join(" / ") ?? "";

export function trackCountLabel(count: number): string {
  const formatted = count.toLocaleString(dateLocale());
  return t(`${formatted} 首`, `${formatted} ${count === 1 ? "track" : "tracks"}`);
}

export function actionButton(iconName: IconName, action: string, label: string): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "icon-button";
  button.dataset.action = action;
  button.setAttribute("aria-label", label);
  button.title = label;
  button.innerHTML = icon(iconName);
  return button;
}

export function empty(text: string): HTMLParagraphElement {
  const p = document.createElement("p");
  p.className = "empty-state";
  p.textContent = text;
  return p;
}

export function makeTrackRow(track: TrackItem, column: PlaylistColumn | null, index: number, overview = false): HTMLDivElement {
  const row = document.createElement("div");
  row.className = "track-row";
  row.dataset.videoId = track.videoId;
  if (column) {
    row.dataset.playlistId = column.playlistId;
    row.dataset.trackIndex = String(index);
    row.dataset.entryId = track.entryId ?? track.videoId + ":" + index;
  }
  const inspect = document.createElement("button");
  inspect.type = "button";
  inspect.className = "track-main";
  inspect.dataset.action = overview ? "locate-result" : "inspect-result";
  inspect.setAttribute("aria-label", overview
    ? t(`定位 ${track.title}`, `Locate ${track.title}`)
    : t(`查看 ${track.title}`, `View ${track.title}`));
  const ordinal = document.createElement("span");
  ordinal.textContent = String(index + 1).padStart(2, "0");
  const cover = document.createElement("img");
  cover.className = "track-cover";
  cover.src = safeAssetUrl(track.thumbnailSquareUrl);
  cover.loading = "lazy";
  cover.alt = "";
  const copy = document.createElement("span");
  copy.className = "track-copy";
  const title = document.createElement("strong");
  title.textContent = track.title;
  title.title = track.title;
  const artists = document.createElement("small");
  artists.textContent = [trackArtists(track), track.isVaultDownloaded ? t("可离线播放", "Available offline") : ""].filter(Boolean).join(" · ");
  artists.title = artists.textContent;
  copy.append(title, artists);
  const time = document.createElement("span");
  time.className = "track-time";
  time.textContent = durationLabel(track.durationSec * 1000);
  inspect.append(overview || !column ? cover : ordinal, copy, time);
  row.append(inspect, actionButton("play", "play-result", t(`播放 ${track.title}`, `Play ${track.title}`)));
  if (overview) row.append(actionButton("locate", "locate-result", t(`定位档案 ${track.title}`, `Locate cassette for ${track.title}`)));
  return row;
}

export function makePlaylistRow(playlist: PlaylistColumn, importable: boolean): HTMLDivElement {
  const row = document.createElement("div");
  row.className = "setting-row";
  row.dataset.playlistId = playlist.playlistId;
  const label = document.createElement("span");
  label.textContent = `${playlist.title} · ${trackCountLabel(playlist.tracks.length)}`;
  row.append(label);
  row.append(actionButton(importable ? "import" : "radar", importable ? "import-result" : "view-playlist",
    importable ? t(`导入 ${playlist.title}`, `Import ${playlist.title}`) : t(`浏览 ${playlist.title}`, `Browse ${playlist.title}`)));
  return row;
}

export function makeArchiveResult(result: ArchiveSearchResult): HTMLDivElement {
  const row = document.createElement("div");
  row.className = "setting-row archive-release";
  const copy = document.createElement("span");
  copy.className = "track-copy";
  const title = document.createElement("strong");
  title.textContent = result.matchingTrack?.title ?? result.title;
  const detail = document.createElement("small");
  detail.textContent = [
    result.matchingTrack?.artist ?? result.artist,
    result.matchingTrack ? result.title : "",
    trackCountLabel(result.trackCount),
  ].filter(Boolean).join(" · ");
  const license = document.createElement("a");
  license.href = result.licenseUrl;
  license.target = "_blank";
  license.rel = "noopener noreferrer";
  license.className = "archive-license";
  license.textContent = result.license;
  copy.append(title, detail, license);
  const importButton = actionButton("import", "import-archive-result", t(`导入 ${result.title}`, `Import ${result.title}`));
  importButton.dataset.sourceUrl = result.sourceUrl;
  if (result.matchingTrack) importButton.dataset.videoId = result.matchingTrack.videoId;
  row.append(copy, importButton);
  return row;
}

export function makeOverviewPlaylist(column: PlaylistColumn, selected: boolean): HTMLElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "overview-playlist";
  button.dataset.action = "choose-overview";
  button.dataset.playlistId = column.playlistId;
  button.setAttribute("aria-current", selected ? "true" : "false");
  const title = document.createElement("span");
  title.textContent = column.title;
  const count = document.createElement("small");
  count.textContent = column.tracks.length.toLocaleString(dateLocale());
  button.append(title, count);
  return button;
}

export class LyricsView {
  readonly root = element("lyrics");
  private lines: LyricLine[] = [];
  private current = -1;
  private manualUntil = 0;
  constructor(signal: AbortSignal, private readonly reduced: () => boolean) {
    const markManual = (): void => this.holdScroll();
    this.root.addEventListener("wheel", markManual, { passive: true, signal });
    this.root.addEventListener("touchmove", markManual, { passive: true, signal });
  }

  loading(): void {
    this.lines = [];
    this.current = -1;
    this.root.replaceChildren(empty(t("正在读取歌词", "Loading lyrics")));
  }

  failure(): void {
    const retry = actionButton("restart", "retry-lyrics", t("重新读取歌词", "Reload lyrics"));
    this.root.replaceChildren(empty(t("无法读取同步歌词", "Could not load synced lyrics")), retry);
  }

  holdScroll(): void {
    this.manualUntil = performance.now() + 2500;
  }

  show(lines: LyricLine[]): void {
    this.lines = [...lines].sort((a, b) => a.startMs - b.startMs);
    this.current = -1;
    this.manualUntil = 0;
    this.root.replaceChildren();
    if (!lines.length) this.root.append(empty(t("此曲暂无同步歌词", "No synced lyrics for this track")));
    for (const line of this.lines) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "lyric-button";
      button.dataset.action = "seek-lyric";
      button.dataset.positionMs = String(line.startMs);
      if (line.duetSide) button.dataset.duet = line.duetSide;
      if (line.isBackgroundVocal) button.dataset.background = "true";
      if (line.words?.length) {
        for (const word of line.words) {
          const span = document.createElement("span");
          span.className = "lyric-word";
          span.textContent = word.word;
          button.append(span);
        }
      } else button.textContent = line.text;
      if (line.translation) {
        const translation = document.createElement("small");
        translation.className = "lyric-translation";
        translation.textContent = line.translation;
        button.append(translation);
      }
      this.root.append(button);
    }
    this.root.scrollTop = 0;
  }

  update(positionMs: number): void {
    const index = this.lines.findIndex(line => positionMs >= line.startMs && positionMs < line.endMs);
    if (index !== this.current) {
      const previous = this.root.children.item(this.current);
      previous?.removeAttribute("aria-current");
      previous?.querySelectorAll(".lyric-word").forEach(word => {
        word.removeAttribute("data-played");
        word.removeAttribute("data-active");
      });
      const row = this.root.children.item(index);
      row?.setAttribute("aria-current", "true");
      if (row && performance.now() > this.manualUntil) {
        const top = (row as HTMLElement).offsetTop + row.clientHeight / 2 - this.root.clientHeight / 2;
        this.root.scrollTo({ top, behavior: this.reduced() ? "instant" : "smooth" });
      }
      this.current = index;
    }
    const words = this.lines[index]?.words;
    if (!words) return;
    this.root.children.item(index)?.querySelectorAll(".lyric-word").forEach((node, wordIndex) => {
      const word = words[wordIndex];
      if (!word) return;
      node.toggleAttribute("data-played", positionMs >= word.endMs);
      node.toggleAttribute("data-active", positionMs >= word.startMs && positionMs < word.endMs);
    });
  }
}

export class SpectrumView {
  private readonly playerBars = Array.from({ length: 32 }, () => this.makeBar(element("spectrum")));

  private makeBar(container: HTMLElement): HTMLElement {
    const bar = document.createElement("i");
    bar.className = "spectrum-bar";
    container.append(bar);
    return bar;
  }

  update(bands: Float32Array | null): void {
    for (let index = 0; index < 32; index++) {
      const band = bands?.[index] ?? 0;
      const height = (9 + band * 84).toFixed(1) + "%";
      const opacity = (.46 + band * .54).toFixed(2);
      const bar = this.playerBars[index];
      if (bar.style.getPropertyValue("--bar-h") === height) continue;
      bar.style.setProperty("--bar-h", height);
      bar.style.setProperty("--bar-opacity", opacity);
    }
  }
}
