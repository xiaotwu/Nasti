import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { language, t } from "./i18n";
import { createMockIpc } from "../../../contracts/mock-ipc";
import type {
  AppUpdateInstallArgs, AppUpdateStatus, AuthorizedCatalog, MediaMode, NastiIpcContract, PlaybackController,
  PlaybackSnapshot, PlaybackSpectrum, PlaybackStatus, PreparedPlayback, YoutubeReply, YoutubeRequest,
} from "../../../contracts/ipc-commands";

function command<T>(name: string, args?: object): Promise<T> {
  return invoke(name, args === undefined ? undefined : { args });
}

export async function setDesktopLanguage(): Promise<void> {
  if (isTauri()) await invoke("ui_set_language", { language: language() });
}

const host: NastiIpcContract = {
  catalog_fetch_columns: () => command("catalog_fetch_columns"),
  catalog_import_playlist_url: args => command("catalog_import_playlist_url", args),
  catalog_preview_playlist: args => command("catalog_preview_playlist", args),
  catalog_apply_playlist_import: args => command("catalog_apply_playlist_import", args),
  catalog_search: args => command("catalog_search", args),
  catalog_search_archive: args => command("catalog_search_archive", args),
  playlist_add_track: args => command("playlist_add_track", args),
  playlist_remove_track: args => command("playlist_remove_track", args),
  playlist_remove_entries: args => command("playlist_remove_entries", args),
  playlist_restore_entries: args => command("playlist_restore_entries", args),
  vault_download_track: args => command("vault_download_track", args),
  vault_remove_track: args => command("vault_remove_track", args),
  vault_get_storage_stats: () => command("vault_get_storage_stats"),
  vault_list_tracks: () => command("vault_list_tracks"),
  vault_set_storage_limit: args => command("vault_set_storage_limit", args),
  lyrics_get_synced: args => command("lyrics_get_synced", args),
  playback_load: args => command("playback_load", args),
  playback_get_state: () => command("playback_get_state"),
  playback_set_playing: args => command("playback_set_playing", args),
  playback_seek: args => command("playback_seek", args),
  playback_set_volume: args => command("playback_set_volume", args),
  playback_list_outputs: () => command("playback_list_outputs"),
  playback_set_output: args => command("playback_set_output", args),
  playback_set_video_frame: args => command("playback_set_video_frame", args),
  playback_prepare: args => command("playback_prepare", args),
  playback_commit: args => command("playback_commit", args),
  playback_cancel_preparation: args => command("playback_cancel_preparation", args),
};

export interface AppUpdater {
  check(): Promise<AppUpdateStatus>;
  install(args: AppUpdateInstallArgs): Promise<void>;
}

export interface TerminalConnection {
  ipc: NastiIpcContract;
  inlineVideo: HTMLVideoElement | null;
  youtube?: (request: YoutubeRequest) => Promise<YoutubeReply>;
  onYoutubeDataCleared?: (clear: () => void) => Promise<() => void>;
  updater?: AppUpdater;
  dispose: () => void;
}

export async function openTerminal(): Promise<TerminalConnection> {
  if (isTauri()) return {
    ipc: host,
    inlineVideo: null,
    youtube: request => command("youtube_playlist_action", request),
    onYoutubeDataCleared: clear => listen("youtube-session-cleared", clear),
    updater: {
      check: () => command("app_update_check"),
      install: args => command("app_update_install", args),
    },
    dispose: () => {},
  };
  const response = await fetch("/authorized-media/catalog.json");
  if (!response.ok) throw new Error(t("授权曲库暂时无法打开", "The authorized library could not be opened."));
  const catalog: AuthorizedCatalog = await response.json();
  const browser = new BrowserPlayback(catalog);
  return { ipc: createMockIpc(catalog, browser), get inlineVideo() { return browser.media; }, dispose: () => browser.dispose() };
}

export const initialPlayback: PlaybackSnapshot = {
  videoId: null, mode: "audio", status: "idle", positionMs: 0, durationMs: 0,
  volume: 1, error: null, spectrum: null,
};

const clamp = (value: number, upper = 1): number => Math.min(upper, Math.max(0, value));

function mediaElement(): HTMLVideoElement {
  const element = document.createElement("video");
  element.playsInline = true;
  element.preload = "auto";
  element.hidden = true;
  element.setAttribute("aria-label", t("音乐影像", "Music video"));
  return element;
}

function waitForMedia(media: HTMLMediaElement, event: "canplay" | "seeked", signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.reject(signal.reason);
  return new Promise((resolve, reject) => {
    const listeners = new AbortController();
    const cleanup = () => { clearTimeout(timeout); listeners.abort(); };
    const fail = () => { cleanup(); reject(new Error(t("授权媒体暂时无法准备", "The authorized media could not be prepared."))); };
    const timeout = window.setTimeout(fail, 10000);
    const options = { signal: listeners.signal, once: true };
    media.addEventListener(event, () => { cleanup(); resolve(); }, options);
    media.addEventListener("error", fail, options);
    signal.addEventListener("abort", () => { cleanup(); reject(signal.reason); }, options);
  });
}

interface Candidate {
  id: number;
  videoId: string;
  mode: MediaMode;
  media: HTMLVideoElement;
  url: string | null;
  controller: AbortController;
}

class BrowserPlayback implements PlaybackController {
  media = mediaElement();
  private mode: MediaMode = "audio";
  private videoId: string | null = null;
  private context: AudioContext | null = null;
  private analyser: AnalyserNode | null = null;
  private source: MediaElementAudioSourceNode | null = null;
  private readonly bins = new Float32Array(2048);
  private readonly pcm = new Float32Array(4096);
  private error: string | null = null;
  private cachedUrl: string | null = null;
  private candidate: Candidate | null = null;

  constructor(private readonly catalog: AuthorizedCatalog) {}

  async playback_load({ videoId, mode, positionMs = 0, autoplay = true }: Parameters<PlaybackController["playback_load"]>[0]): Promise<PlaybackSnapshot> {
    this.cancelCandidate();
    const path = this.mediaPath(videoId, mode);
    if (autoplay) this.resumeAnalysis();

    const replace = this.mode !== mode || this.videoId !== videoId || Boolean(this.media.error);
    this.error = null;
    this.mode = mode;
    this.videoId = videoId;
    if (replace) {
      this.media.pause();
      if (this.cachedUrl) URL.revokeObjectURL(this.cachedUrl);
      this.cachedUrl = await this.findCachedMedia(path);
      this.media.src = this.cachedUrl ?? path;
      this.media.onloadedmetadata = positionMs > 0 ? () => {
        this.media.onloadedmetadata = null;
        this.seek(positionMs);
      } : null;
    } else {
      this.seek(positionMs);
    }
    if (autoplay) await this.play();
    return this.snapshot();
  }

  async playback_get_state(): Promise<PlaybackSnapshot> {
    return this.snapshot();
  }

  async playback_set_playing({ playing }: { playing: boolean }): Promise<PlaybackSnapshot> {
    if (playing) {
      this.resumeAnalysis();
      if (this.media.ended) this.seek(0);
      await this.play();
    } else this.media.pause();
    return this.snapshot();
  }

  async playback_seek({ positionMs }: { positionMs: number }): Promise<PlaybackSnapshot> {
    this.seek(positionMs);
    return this.snapshot();
  }

  async playback_set_volume({ volume }: { volume: number }): Promise<PlaybackSnapshot> {
    this.media.volume = clamp(volume);
    return this.snapshot();
  }

  async playback_set_video_frame(): Promise<void> {}

  async playback_prepare({ switchId, videoId, mode }: Parameters<PlaybackController["playback_prepare"]>[0]): Promise<void> {
    this.cancelCandidate();
    const path = this.mediaPath(videoId, mode);
    const candidate: Candidate = { id: switchId, videoId, mode, media: mediaElement(), url: null, controller: new AbortController() };
    this.candidate = candidate;
    const { media, controller } = candidate;
    const url = await this.findCachedMedia(path);
    if (controller.signal.aborted) {
      if (url) URL.revokeObjectURL(url);
      throw controller.signal.reason;
    }
    candidate.url = url;
    try {
      const ready = waitForMedia(media, "canplay", controller.signal);
      media.src = url ?? path;
      await ready;
    } catch (error) {
      if (this.candidate === candidate) this.cancelCandidate();
      throw error;
    }
  }

  async playback_commit({ switchId, alignTime, positionMs, autoplay }: Parameters<PlaybackController["playback_commit"]>[0]): Promise<PreparedPlayback> {
    const candidate = this.candidate;
    if (!candidate || candidate.id !== switchId) throw new Error(t("媒体切换已取消", "The media switch was canceled."));
    const { media, controller } = candidate;
    const before = this.snapshot();
    const aligned = alignTime && before.videoId === candidate.videoId && before.status !== "ended";
    const ms = aligned ? before.positionMs : positionMs;
    const target = clamp(ms / 1000, media.duration);
    if (Math.abs(media.currentTime - target) > .005) {
      const ready = waitForMedia(media, "seeked", controller.signal);
      media.currentTime = target;
      await ready;
    }
    if (controller.signal.aborted) throw controller.signal.reason;
    const playing = autoplay ?? !this.media.paused;
    if (playing) {
      media.muted = true;
      await media.play();
    }
    if (controller.signal.aborted) throw controller.signal.reason;
    const previous = this.snapshot();
    const old = this.media;
    old.pause();
    media.volume = old.volume;
    if (aligned && Math.abs(media.currentTime - previous.positionMs / 1000) > .08) {
      media.currentTime = clamp(previous.positionMs / 1000, media.duration);
    }
    this.source?.disconnect();
    this.source = null;
    this.release(old, this.cachedUrl);
    this.media = media;
    this.mode = candidate.mode;
    this.videoId = candidate.videoId;
    this.error = null;
    this.cachedUrl = candidate.url;
    this.candidate = null;
    if (playing) this.resumeAnalysis();
    media.muted = false;
    return { playback: this.snapshot(), previous };
  }

  async playback_cancel_preparation({ switchId }: { switchId: number }): Promise<void> {
    if (this.candidate?.id === switchId) this.cancelCandidate();
  }

  dispose(): void {
    this.cancelCandidate();
    this.release(this.media, this.cachedUrl);
    this.source?.disconnect();
    void this.context?.close();
  }

  private mediaPath(videoId: string, mode: MediaMode): string {
    const track = this.catalog.tracks.find(item => item.videoId === videoId);
    const path = mode === "audio" ? track?.audioUrl : track?.mvUrl;
    if (!path?.startsWith("/authorized-media/")) throw new Error(t("此版本没有可播放的授权文件", "This version has no authorized file to play."));
    return path;
  }

  private cancelCandidate(): void {
    const candidate = this.candidate;
    if (!candidate) return;
    this.candidate = null;
    candidate.controller.abort();
    this.release(candidate.media, candidate.url);
  }

  private release(media: HTMLMediaElement, url: string | null): void {
    media.pause();
    media.removeAttribute("src");
    media.load();
    media.remove();
    if (url) URL.revokeObjectURL(url);
  }

  private async play(): Promise<void> {
    try {
      await this.media.play();
    } catch (error) {
      this.error = error instanceof Error ? error.message : String(error);
      throw error;
    }
  }

  private async findCachedMedia(path: string): Promise<string | null> {
    try {
      const response = await globalThis.caches?.match(path, { cacheName: "nasti-authorized-vault-v1" });
      return response ? URL.createObjectURL(await response.blob()) : null;
    } catch {
      return null;
    }
  }

  private seek(positionMs: number): void {
    if (!Number.isFinite(this.media.duration) || !Number.isFinite(positionMs)) return;
    this.media.currentTime = clamp(positionMs / 1000, this.media.duration);
  }

  private resumeAnalysis(): void {
    try {
      if (!this.context) {
        this.context = new AudioContext();
        this.analyser = this.context.createAnalyser();
        this.analyser.fftSize = 4096;
        this.analyser.smoothingTimeConstant = .63;
        this.analyser.connect(this.context.destination);
      }
      if (!this.source && this.analyser) {
        this.source = this.context.createMediaElementSource(this.media);
        this.source.connect(this.analyser);
      }
      if (this.context.state === "suspended") void this.context.resume().catch(() => {});
    } catch {
      this.analyser = null;
    }
  }

  private status(): PlaybackStatus {
    if (!this.videoId) return "idle";
    if (this.media.error || this.error) return "error";
    if (this.media.ended) return "ended";
    if (this.media.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) return "loading";
    return this.media.paused ? "paused" : "playing";
  }

  private snapshot(): PlaybackSnapshot {
    const status = this.status();
    return {
      videoId: this.videoId,
      mode: this.mode,
      status,
      positionMs: Math.round(this.media.currentTime * 1000),
      durationMs: Number.isFinite(this.media.duration) ? Math.round(this.media.duration * 1000) : 0,
      volume: this.media.volume,
      error: this.media.error?.message ?? this.error,
      spectrum: status === "playing" && !document.hidden ? this.spectrum() : null,
    };
  }

  private spectrum(): PlaybackSpectrum | null {
    if (!this.context || !this.analyser || this.context.state !== "running") return null;
    this.analyser.getFloatFrequencyData(this.bins);
    this.analyser.getFloatTimeDomainData(this.pcm);
    const resolution = this.context.sampleRate / this.analyser.fftSize;
    const band = (low: number, high: number): number => {
      const first = Math.max(1, Math.floor(low / resolution));
      const end = Math.min(this.bins.length, Math.max(first + 1, Math.ceil(high / resolution)));
      let peak = -100;
      for (let i = first; i < end; i++) peak = Math.max(peak, this.bins[i]);
      return clamp((peak + 92) / 62);
    };
    const bars32 = Array.from({ length: 32 }, (_, i) => band(27 * 740 ** (i / 32), 27 * 740 ** ((i + 1) / 32)));
    const rms = Math.sqrt(this.pcm.reduce((sum, sample) => sum + sample ** 2, 0) / this.pcm.length);
    return { rms: clamp(rms * 2.6), subBass: band(20, 80), midRange: band(250, 2000), highPresence: band(4000, 16000), bars32 };
  }
}
