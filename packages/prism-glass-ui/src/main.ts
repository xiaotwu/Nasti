import { createSkyframeScene } from "@nasti/skyframe-3d";
import type {
  MediaMode, PlaybackSnapshot, PlaybackVideoFrame, PreparedPlayback,
  PlaylistColumn, ThemeVariant, TrackItem,
} from "../../../contracts/ipc-commands";
import type { DetailSubTab, PlaylistOccurrence, SceneSelection, SceneViewMode } from "../../../contracts/scene-bridge";
import { BootSequence } from "./boot";
import { icon, setIcon, type IconName } from "./icons";
import { language, localizeError, localizePlaylist, localizeTrack, selectLanguage, t } from "./i18n";
import { durationLabel, element, mountLayout, safeAssetUrl, setText } from "./layout";
import { LibraryPanels } from "./library-panels";
import { installGlassMotion } from "./materials/glass";
import { OutputPicker } from "./output-picker";
import { initialPlayback, openTerminal, setDesktopLanguage, type TerminalConnection, type TerminalIpc } from "./playback";
import { SettingsNavigation, SettingsUpdates } from "./settings";
import { type ShortcutAction } from "./shortcut-bindings";
import { ApplicationShortcuts } from "./shortcuts";
import { nextRepeatMode, occurrenceId, repeatMode, TrackNavigation } from "./track-navigation";
import { actionButton, empty, LyricsView, makeOverviewPlaylist, makePlaylistRow, makeTrackRow, SpectrumView, trackArtists } from "./views";
import { bindVolumeRanges } from "./volume-range";
import { playlistReference, YoutubePanel, type ToastLevel } from "./youtube-panel";
import "./styles.css";
import "./archive-navigation.css";
import "./transport.css";

type Location = { column: PlaylistColumn; index: number; track: TrackItem };
type Panel = "search" | "settings";
type Position = PlaylistOccurrence;
type SearchFilter = "all" | "songs" | "videos" | "playlists";
type MediaSwitch = { id: number; mode: MediaMode; target: Location };
type VideoSession = { target: Location; audioTarget: Location; resumeMs: number; pauseOnExit: boolean };
type TerminalState = {
  inspected: Location | null;
  queue: Position | null;
  detachedTrack: TrackItem | null;
  playback: PlaybackSnapshot;
  mode: SceneViewMode;
  tab: DetailSubTab;
  videoSession: VideoSession | null;
  lastVolume: number;
  navigation: TrackNavigation;
};

const tabNames: DetailSubTab[] = ["tracklist", "lyrics", "mv-stage"];
const searchFilters: SearchFilter[] = ["all", "songs", "videos", "playlists"];
const presets = ["performance", "balanced", "high", "ultra"] as const;
const maxArchiveTicks = 32;

function position({ column, track, index }: Location): Position {
  return { playlistId: column.playlistId, entryId: occurrenceId(track, index) };
}

function storePreference(key: string, value?: string): string | null {
  try {
    if (value !== undefined) localStorage.setItem("nasti." + key, value);
    return localStorage.getItem("nasti." + key);
  } catch {
    return null;
  }
}

class Terminal {
  private readonly ipc: TerminalIpc;
  private readonly library: LibraryPanels;
  private readonly youtube: YoutubePanel;
  private readonly outputs: OutputPicker;
  private readonly events = new AbortController();
  private readonly shell = element("terminal");
  private readonly scene = createSkyframeScene({
    archive: t("歌曲档案阵列", "Track archive"),
    detail: t("播放详情档案", "Playback detail"),
    inspection: t("360° 档案拆解", "360° cassette inspection"),
  });
  private readonly panel = element<HTMLDialogElement>("panel");
  private readonly seek = element<HTMLInputElement>("seek");
  private readonly volume = element<HTMLInputElement>("volume");
  private readonly mvViewport = element("mv-viewport");
  private readonly reduced = matchMedia("(prefers-reduced-motion: reduce)");
  private readonly systemTheme = matchMedia("(prefers-color-scheme: dark)");
  private readonly glass = installGlassMotion(this.shell);
  private readonly lyrics = new LyricsView(this.events.signal, () => this.motionReduced);
  private readonly spectrum = new SpectrumView();
  private readonly boot = new BootSequence();
  private readonly settings = new SettingsNavigation(this.events.signal);
  private readonly shortcuts = new ApplicationShortcuts(this.events.signal, () => this.renderPlayback());
  private readonly settingsVolume = element<HTMLInputElement>("settings-volume");
  private readonly outputVolume = element<HTMLInputElement>("output-volume");
  private readonly volumeControls = [this.volume, this.settingsVolume, this.outputVolume];
  private readonly frameResize = new ResizeObserver(() => this.updateVideoFrame());
  private readonly mvPoster = element<HTMLImageElement>("mv-poster");
  private columns: PlaylistColumn[] = [];
  private inspected: Location | null = null;
  private queue: Position | null = null;
  private detachedTrack: TrackItem | null = null;
  private playback = initialPlayback;
  private navigation = new TrackNavigation();
  private shuffle = storePreference("playback-shuffle") === "on";
  private repeat = repeatMode(storePreference("playback-repeat"));
  private mode: SceneViewMode = "archive-array";
  private tab: DetailSubTab = "lyrics";
  private theme: ThemeVariant = "banshee-obsidian";
  private themeSetting = storePreference("theme") ?? "system";
  private silhouette = storePreference("silhouette") !== "off";
  private quality = presets.find(value => value === storePreference("quality")) ?? "balanced";
  private reduceMotion = storePreference("motion") === "reduced";
  private wrapTracks = storePreference("track-wrap") !== "stop";
  private wrapLists = storePreference("list-wrap") !== "stop";
  private advanceAfterEnd = storePreference("playback-end") !== "stop";
  private overviewId: string | null = null;
  private filter: SearchFilter = "all";
  private power = storePreference("power") === "on";
  private panelTrigger: HTMLElement | null = null;
  private libraryReady = false;
  private sceneReady = false;
  private sceneFailed = false;
  private blockSelections = false;
  private exploded = false;
  private detailColumn: PlaylistColumn | null = null;
  private listOpened = false;
  private lyricTrack: string | null = null;
  private inspectionScroll = 0;
  private closed = false;
  private scrubbing = false;
  private volumeDraft: number | null = null;
  private queuedVolume: number | null = null;
  private volumeCommit: Promise<void> | null = null;
  private volumeRevision = 0;
  private lastVolume = 1;
  private lyricRequest = 0;
  private catalogRevision = 0;
  private searchRequest = 0;
  private searchTimer = 0;
  private searchTracks = new Map<string, TrackItem>();
  private readonly toastTimers = new Set<number>();
  private pollTimer = 0;
  private videoKey = "";
  private pendingVideoFrame = Promise.resolve();
  private pendingPlayback = Promise.resolve();
  private pendingPlaybackCount = 0;
  private playbackVersion = 0;
  private switchCount = 0;
  private mediaSwitch: MediaSwitch | null = null;
  private videoSession: VideoSession | null = null;
  private videoError = "";
  private returningAudio = false;
  private startingVideo = false;
  private videoIntent = 0;
  private transportActionKey = "";

  constructor(private readonly connection: TerminalConnection, restored?: TerminalState) {
    if (restored) {
      this.inspected = restored.inspected;
      this.queue = restored.queue;
      this.detachedTrack = restored.detachedTrack ? localizeTrack(restored.detachedTrack) : null;
      this.playback = restored.playback;
      this.mode = restored.mode;
      this.videoSession = restored.videoSession;
      this.lastVolume = restored.lastVolume;
      this.navigation = restored.navigation;
    }
    this.ipc = connection.ipc;
    if (connection.updater) new SettingsUpdates(connection.updater, this.events.signal);
    this.outputs = new OutputPicker(this.ipc, this.events.signal);
    this.youtube = new YoutubePanel(connection.youtube, async playlistId => {
      if (playlistId) this.overviewId = playlistId;
      await this.reloadCatalog();
    }, (message, level, openLogin) => this.message(message, level, openLogin), this.events.signal);
    this.library = new LibraryPanels(this.ipc, () => this.reloadCatalog(),
      track => this.playVaultTrack(track), (value, level) => this.message(value, level), this.events.signal);
    this.shell.dataset.native = String(!connection.inlineVideo);
    if (connection.inlineVideo) {
      connection.inlineVideo.hidden = true;
      this.mvViewport.append(connection.inlineVideo);
    }
    this.connectEvents();
    void connection.onYoutubeSnapshotsExpired?.(() => {
      void this.reloadCatalog().catch(error => this.message(localizeError(error), "error"));
    }).then(unlisten => {
      if (this.events.signal.aborted) unlisten();
      else this.events.signal.addEventListener("abort", unlisten, { once: true });
    }).catch(() => {});
    this.frameResize.observe(this.mvViewport);
    this.frameResize.observe(element("transport-menu"));
    this.frameResize.observe(element("output-panel"));
    this.frameResize.observe(element("volume-panel"));
    this.loadSettings();
    this.setTab(restored?.tab ?? "lyrics");
    this.renderPlaybackModes();
    this.renderPlayback();
  }

  async start(reopenSettings = false): Promise<void> {
    this.setMode(this.mode);
    const bootComplete = reopenSettings ? Promise.resolve() : new Promise<void>(resolve => {
      this.boot.start(this.motionReduced, () => {
        if (document.hidden) resolve();
        else window.requestAnimationFrame(() => window.requestAnimationFrame(() => resolve()));
      });
    });
    void this.youtube.start(bootComplete, !reopenSettings);
    // WebGL initialization can block the first paint, so start it after the opening cue.
    const sceneInit = bootComplete.then(() => {
      if (!this.closed) return this.scene.mount(element("scene"));
    }).then(() => {
      if (this.closed) return;
      this.sceneReady = true;
      this.scene.setSelectionListener(selection => this.onSceneSelection(selection));
      this.scene.setTheme(this.theme);
      this.scene.setReducedMotion(this.motionReduced);
      this.scene.setArchiveNavigation(this.wrapLists, this.wrapTracks);
      this.scene.setQualityPreset(this.quality);
      this.syncSceneSelection();
      this.setMode(this.mode);
    }).catch(() => {
      if (this.closed) return;
      this.sceneFailed = true;
      this.shell.dataset.fallback = "true";
      if (this.mode !== "boot") this.setMode("radar-grid");
    });
    const loading = element("loading-status");
    const load = async () => {
      loading.hidden = false;
      setText(loading, t("正在打开曲库", "Opening library…"));
      try {
        await Promise.all([sceneInit, this.reloadCatalog()]);
        loading.hidden = true;
        if (!reopenSettings && !this.panel.open && document.activeElement === document.body) this.focusArchive();
      } catch {
        const retry = actionButton("restart", "retry-library", t("重新打开曲库", "Retry opening the library"));
        retry.onclick = () => void load();
        loading.replaceChildren(document.createTextNode(t("曲库打开失败 ", "Could not open the library. ")), retry);
      }
    };
    await load();
    void this.poll();
    if (reopenSettings && !this.closed) {
      await this.showPanel("settings");
      if (!this.closed) element("language-setting").focus({ preventScroll: true });
    }
  }

  private connectEvents(): void {
    const { signal } = this.events;
    this.shell.addEventListener("click", event => {
      if (!(event.target instanceof Element)) return;
      const button = event.target.closest<HTMLButtonElement>("button[data-action]");
      if (button && !button.disabled) void this.handleAction(button).catch(error => this.message(localizeError(error), "error"));
    }, { signal });
    this.panel.addEventListener("close", () => {
      this.searchRequest++;
      if (!this.panel.open) this.updatePower();
    }, { signal });
    element("search-input").addEventListener("input", () => {
      this.searchRequest++;
      clearTimeout(this.searchTimer);
      const value = element<HTMLInputElement>("search-input").value.trim();
      const reference = playlistReference(value);
      element("search-results").replaceChildren();
      if (reference || value.length >= 2) {
        this.searchTimer = window.setTimeout(() => void this.search(), reference ? 180 : 700);
      }
    }, { signal });
    element("search-form").addEventListener("submit", event => {
      event.preventDefault();
      clearTimeout(this.searchTimer);
      void this.search();
    }, { signal });
    this.seek.addEventListener("input", () => {
      this.scrubbing = true;
      setText(element("elapsed"), durationLabel(this.seek.valueAsNumber));
      this.describeSeek(this.seek.valueAsNumber, Number(this.seek.max));
      this.setRange(this.seek, this.seek.valueAsNumber, Number(this.seek.max));
    }, { signal });
    this.seek.addEventListener("change", () => {
      const value = this.seek.valueAsNumber;
      void this.playbackAction(() => this.ipc.playback_seek({ positionMs: value })).finally(() => {
        this.scrubbing = false;
        this.renderPlayback();
      });
    }, { signal });
    this.seek.addEventListener("pointercancel", () => { this.scrubbing = false; }, { signal });
    bindVolumeRanges(this.volumeControls, volume => { void this.commitVolume(volume); }, signal);
    const scale = element<HTMLInputElement>("archive-scale");
    const tickMarkers = element("archive-tick-markers");
    scale.addEventListener("input", () => {
      const target = this.locate(this.inspected?.column.playlistId, scale.valueAsNumber);
      if (target) this.select(target, false);
    }, { signal });
    scale.addEventListener("pointermove", event => {
      const bounds = tickMarkers.getBoundingClientRect();
      const offset = Math.min(1, Math.max(0, (event.clientX - bounds.left) / bounds.width));
      const index = Math.round(offset * Number(scale.max));
      for (const marker of tickMarkers.children) {
        marker.classList.toggle("is-hovered", (marker as HTMLElement).dataset.index === String(index));
      }
    }, { signal });
    scale.addEventListener("pointerleave", () => {
      tickMarkers.querySelector(".is-hovered")?.classList.remove("is-hovered");
    }, { signal });
    scale.addEventListener("keydown", event => {
      const direction = { ArrowLeft: -1, ArrowDown: -1, ArrowRight: 1, ArrowUp: 1 }[event.key];
      if (direction === undefined) return;
      event.preventDefault();
      this.scene.stepArchive(0, direction);
    }, { signal });
    element("settings-panel").addEventListener("change", event => {
      if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) {
        this.updateSetting(event.target);
      }
    }, { signal });
    const dismissPlayerPopups = (target: EventTarget | null) => {
      if (!(target instanceof Element)) return;
      if (!target.closest("#transport-more")) this.closeTransportMenu();
      if (!target.closest("#volume-anchor")) this.closeVolumePanel(false);
      if (!target.closest("#output-anchor")) this.outputs.close();
    };
    document.addEventListener("click", event => dismissPlayerPopups(event.target), { signal, capture: true });
    element("transport-dock").addEventListener("focusout", event => dismissPlayerPopups(event.relatedTarget), { signal });
    document.addEventListener("keydown", this.onKey, { signal, capture: true });
    document.addEventListener("visibilitychange", () => this.updatePower(), { signal });
    window.addEventListener("blur", () => this.updateVideoFrame(), { signal });
    window.addEventListener("resize", () => this.updatePower(), { signal, passive: true });
    window.addEventListener("scroll", () => this.updateVideoFrame(), { signal, passive: true, capture: true });
    for (const type of ["online", "offline"] as const) window.addEventListener(type, () => this.updateVideoFrame(), { signal });
    this.reduced.addEventListener("change", () => {
      if (this.reduced.matches && this.boot.active) this.boot.skip();
      this.syncMotion();
      this.updatePower();
    }, { signal });
    this.systemTheme.addEventListener("change", () => this.applyTheme(), { signal });
    window.addEventListener("pagehide", () => this.dispose(), { signal, once: true });
  }

  private readonly onKey = (event: KeyboardEvent): void => {
    if (this.boot.active) {
      if (event.key === "Escape") {
        event.preventDefault();
        this.boot.skip();
      }
      return;
    }
    if (this.shortcuts.record(event)) return;
    if (event.key === "Escape" && !(event.ctrlKey || event.metaKey || event.altKey)) {
      event.preventDefault();
      if (!element("volume-panel").hidden) this.toggleVolume();
      else if (this.outputs.open) this.outputs.close(true);
      else if (this.youtube.dismiss()) return;
      else if (!element("transport-menu").hidden) this.closeTransportMenu(true);
      else if (this.panel.open) this.closePanel();
      else if (this.mode === "disassembly-360") this.leaveInspection();
      else if (this.mode === "detail-extracted") this.closeDetail();
      else if (this.mode === "radar-grid") this.toggleRadar();
      return;
    }
    if (event.defaultPrevented) return;
    const action = this.shortcuts.match(event);
    if (!action) return;
    const target = event.target instanceof Element ? event.target : null;
    const modified = event.ctrlKey || event.metaKey || event.altKey;
    const editing = target?.closest("input,textarea,select,[contenteditable],[role=textbox]");
    if (editing && !(event.ctrlKey || event.metaKey)) return;
    if (!modified && (this.panel.open || (event.key === " " && target?.closest("button,a[href]")))) return;
    event.preventDefault();
    if (!event.repeat) this.runShortcut(action);
  };

  private runShortcut(action: ShortcutAction): void {
    switch (action) {
      case "search": case "settings": void this.showPanel(action); return;
      case "library": void this.showVaultSettings(); return;
      case "radar": this.toggleRadar(); return;
      case "play-toggle": void this.togglePlaying(); return;
      case "previous": case "next": void this.changeTrack(action === "previous" ? -1 : 1, true); return;
      case "mute": void this.mute(); return;
      case "seek-backward": case "seek-forward": {
        if (!this.playback.videoId || !this.playback.durationMs) return;
        const offset = action === "seek-forward" ? 5000 : -5000;
        void this.playbackAction(() => this.ipc.playback_seek({
          positionMs: Math.max(0, Math.min(this.playback.positionMs + offset, this.playback.durationMs)),
        }));
        return;
      }
      case "volume-down": case "volume-up": {
        const offset = action === "volume-up" ? .05 : -.05;
        const current = this.volumeDraft ?? this.playback.volume;
        void this.commitVolume(Math.max(0, Math.min(1, current + offset)));
        return;
      }
    }
  }

  private async handleAction(button: HTMLButtonElement): Promise<void> {
    switch (button.dataset.action) {
      case "search": return this.showPanel("search");
      case "retry-vault": return this.library.showVault();
      case "play-vault": case "vault-menu": case "remove-vault-copy": return this.library.handleVault(button.dataset.action, button);
      case "settings": return this.showPanel("settings");
      case "settings-section": this.settings.jump(button.dataset.section, this.motionReduced); return;
      case "close-panel": this.closePanel(); return;
      case "radar": this.toggleRadar(); return;
      case "previous-playlist": this.scene.stepArchive(-1, 0); return;
      case "next-playlist": this.scene.stepArchive(1, 0); return;
      case "choose-overview": this.overviewId = button.dataset.playlistId ?? null; this.renderOverview(true); return;
      case "locate-result": {
        const target = this.locateButton(button);
        if (target) {
          this.select(target, false);
          this.focusArchive();
        }
        return;
      }
      case "play-toggle": return this.togglePlaying();
      case "previous": return this.changeTrack(-1, true);
      case "next": return this.changeTrack(1, true);
      case "shuffle-tracks":
        this.shuffle = !this.shuffle;
        storePreference("playback-shuffle", this.shuffle ? "on" : "off");
        this.navigation.reset();
        this.renderPlaybackModes();
        return;
      case "repeat-tracks":
        this.repeat = nextRepeatMode(this.repeat);
        storePreference("playback-repeat", this.repeat);
        this.renderPlaybackModes();
        return;
      case "volume-panel": this.toggleVolume(); return;
      case "output-panel":
        this.closeVolumePanel(false);
        this.closeTransportMenu();
        return this.outputs.toggle();
      case "transport-more": this.toggleTransportMenu(); return;
      case "mute":
        button.focus({ preventScroll: true });
        return this.mute();
      case "tracklist": case "lyrics": case "mv-stage": this.setTab(button.dataset.action); return;
      case "filter-all": return this.changeFilter("all");
      case "filter-songs": return this.changeFilter("songs");
      case "filter-videos": return this.changeFilter("videos");
      case "filter-playlists": return this.changeFilter("playlists");
      case "close-detail": this.closeDetail(); return;
      case "inspect-result": {
        const target = this.locateButton(button);
        if (!target) return this.message(t("请先导入此曲所属的歌单", "Import the playlist that contains this track first."));
        this.closePanel();
        this.select(target, true);
        return;
      }
      case "play-result": {
        const target = this.locateButton(button);
        if (!target) return this.message(t("请先导入此曲所属的歌单", "Import the playlist that contains this track first."));
        const key = position(target);
        const video = button.closest<HTMLElement>("[data-mode]")?.dataset.mode === "video" && target.track.mvVideoId;
        if (!video && this.playback.mode === "audio" && !["error", "idle"].includes(this.playback.status)
          && key.playlistId === this.queue?.playlistId && key.entryId === this.queue?.entryId) {
          return this.togglePlaying();
        }
        if (this.mode === "radar-grid" && !this.panel.open) return this.playTrack(target);
        this.closePanel();
        this.select(target, true);
        if (!video) return this.playTrack(target);
        this.setTab("mv-stage");
        return this.startVideo();
      }
      case "start-mv": return this.startVideo();
      case "toggle-vault": return this.toggleVault();
      case "seek-lyric": return this.seekLyric(Number(button.dataset.positionMs));
      case "view-playlist": {
        this.overviewId = button.closest<HTMLElement>("[data-playlist-id]")?.dataset.playlistId ?? null;
        this.closePanel();
        this.setMode("radar-grid");
        return;
      }
      case "import-result": {
        const playlistId = button.closest<HTMLElement>("[data-playlist-id]")?.dataset.playlistId;
        if (playlistId) {
          element<HTMLInputElement>("search-input").value = playlistId;
          await this.search(playlistId);
        }
        return;
      }
      case "retry-lyrics": if (this.inspected) return this.loadLyrics(this.inspected.track.videoId); return;
      case "disassemble": {
        const active = this.transportLocation();
        if (active && (!this.inspected || active.track !== this.inspected.track || this.mode !== "detail-extracted")) this.select(active, true);
        this.enterInspection();
        return;
      }
      case "leave-inspection": this.leaveInspection(); return;
      case "explode": case "assemble": this.expandInspection(button.dataset.action === "explode"); return;
      case "reset-view": this.scene.resetView(); return;
      case "replay-boot": this.boot.start(this.motionReduced, () => button.focus({ preventScroll: true })); return;
      case "skip-boot": this.boot.skip(); return;
    }
  }

  private get motionReduced(): boolean {
    return this.reduceMotion || this.reduced.matches;
  }

  private syncMotion(): void {
    this.shell.dataset.reduced = String(this.motionReduced);
    this.scene.setReducedMotion(this.motionReduced);
  }

  private loadSettings(): void {
    if (!["system", "celestial-cyan", "banshee-obsidian"].includes(this.themeSetting)) this.themeSetting = "system";
    const values = {
      language: language(),
      theme: this.themeSetting,
      end: this.advanceAfterEnd ? "continue" : "stop",
      "track-wrap": this.wrapTracks ? "loop" : "stop",
      "list-wrap": this.wrapLists ? "loop" : "stop",
      motion: this.reduceMotion ? "reduced" : "standard",
      quality: this.quality,
    };
    for (const [id, value] of Object.entries(values)) element<HTMLSelectElement>(id + "-setting").value = value;
    element<HTMLInputElement>("silhouette-setting").checked = this.silhouette;
    element<HTMLInputElement>("power-setting").checked = this.power;
    this.applyTheme();
    this.syncMotion();
  }

  private updateSetting(control: HTMLInputElement | HTMLSelectElement): void {
    const { value } = control;
    switch (control.id) {
      case "language-setting": return this.changeLanguage(value);
      case "theme-setting": this.themeSetting = value; storePreference("theme", value); this.applyTheme(); break;
      case "end-setting": this.advanceAfterEnd = value === "continue"; storePreference("playback-end", value); break;
      case "track-wrap-setting": this.wrapTracks = value === "loop"; storePreference("track-wrap", value); break;
      case "list-wrap-setting": this.wrapLists = value === "loop"; storePreference("list-wrap", value); break;
      case "motion-setting": this.reduceMotion = value === "reduced"; storePreference("motion", value); this.syncMotion(); break;
      case "quality-setting": {
        const preset = presets.find(item => item === value);
        if (!preset) break;
        this.quality = preset;
        storePreference("quality", preset);
        this.scene.setQualityPreset(preset);
        break;
      }
      case "silhouette-setting": {
        this.silhouette = (control as HTMLInputElement).checked;
        storePreference("silhouette", this.silhouette ? "on" : "off");
        break;
      }
      case "power-setting": {
        this.power = (control as HTMLInputElement).checked;
        storePreference("power", this.power ? "on" : "off");
        break;
      }
    }
    this.scene.setArchiveNavigation(this.wrapLists, this.wrapTracks);
    this.renderArchiveCaption();
    this.updatePower();
  }

  private changeLanguage(value: string): void {
    if (!selectLanguage(value)) return;
    const restored: TerminalState = {
      inspected: this.inspected, queue: this.queue, detachedTrack: this.detachedTrack,
      playback: this.playback, mode: this.mode, tab: this.tab,
      videoSession: this.videoSession, lastVolume: this.lastVolume, navigation: this.navigation,
    };
    this.dispose(false);
    renderLayout();
    void new Terminal(this.connection, restored).start(true);
  }

  private applyTheme(): void {
    this.theme = this.themeSetting === "system" ? (this.systemTheme.matches ? "banshee-obsidian" : "celestial-cyan")
      : this.themeSetting === "banshee-obsidian" ? "banshee-obsidian" : "celestial-cyan";
    this.shell.dataset.theme = this.theme;
    if (this.sceneReady) this.scene.setTheme(this.theme);
  }

  private buttons(action: string): NodeListOf<HTMLButtonElement> {
    return this.shell.querySelectorAll<HTMLButtonElement>('button[data-action="' + action + '"]');
  }

  private pressed(action: string, value: boolean): void {
    for (const button of this.buttons(action)) button.setAttribute("aria-pressed", String(value));
  }

  private disable(action: string, value: boolean): void {
    for (const button of this.buttons(action)) button.disabled = value;
  }

  private locate(playlistId: string | undefined, index: number): Location | null {
    const column = this.columns.find(column => column.playlistId === playlistId);
    const track = column?.tracks[index];
    return column && track ? { column, index, track } : null;
  }

  private findByVideoId(id: string | null | undefined): Location | null {
    if (!id) return null;
    for (const column of this.columns) {
      const index = column.tracks.findIndex(track => track.videoId === id);
      if (index >= 0) return { column, index, track: column.tracks[index] };
    }
    return null;
  }

  private locateButton(button: HTMLButtonElement): Location | null {
    const row = button.closest<HTMLElement>("[data-video-id]");
    if (row?.dataset.playlistId && row.dataset.entryId) return this.locatePosition({ playlistId: row.dataset.playlistId, entryId: row.dataset.entryId });
    const result = row?.closest("#search-results") && this.searchTracks.get(`${row.dataset.mode === "video" ? "video" : "audio"}:${row.dataset.videoId ?? ""}`);
    if (result) {
      const present = this.findByVideoId(result.videoId);
      return present ? { ...present, track: { ...present.track, mvVideoId: result.mvVideoId ?? present.track.mvVideoId } }
        : this.detachedLocation(result);
    }
    return this.locate(row?.dataset.playlistId, Number(row?.dataset.trackIndex)) ?? this.findByVideoId(row?.dataset.videoId);
  }

  private detachedLocation(track: TrackItem): Location {
    return { index: 0, track, column: {
      playlistId: "", columnIndex: -1, title: track.title, codeName: "", isEditable: false, tracks: [track],
    } };
  }

  private locatePosition(key: Position | null): Location | null {
    if (!key) return null;
    const column = this.columns.find(column => column.playlistId === key.playlistId);
    const index = column?.tracks.findIndex((track, index) => occurrenceId(track, index) === key.entryId) ?? -1;
    return this.locate(key.playlistId, index);
  }

  private async reloadCatalog(): Promise<void> {
    const request = ++this.catalogRevision;
    const columns = (await this.ipc.catalog_fetch_columns()).map(localizePlaylist).sort((a, b) => a.columnIndex - b.columnIndex);
    if (this.closed || request !== this.catalogRevision) return;
    const old = this.inspected;
    this.columns = columns;
    this.libraryReady = true;
    const oldList = this.columns.find(column => column.playlistId === old?.column.playlistId);
    const next = (old?.column.columnIndex === -1 ? old : null) ?? this.locatePosition(old && position(old))
      ?? this.locate(oldList?.playlistId, Math.min(old?.index ?? 0, (oldList?.tracks.length ?? 0) - 1))
      ?? this.findByVideoId(old?.track.videoId)
      ?? this.locate(this.columns.find(column => column.tracks.length > 0)?.playlistId, 0);
    if (next) this.updateInspected(next);
    else {
      this.leaveVideo();
      this.closeTransportMenu();
      this.inspected = null;
    }
    this.syncSceneSelection();
    this.renderOverview();
    if (this.inspected) this.renderSelected();
    this.setMode(this.mode, false);
  }

  private syncSceneSelection(): void {
    if (!this.sceneReady || !this.libraryReady) return;
    this.blockSelections = true;
    this.scene.setColumns(this.columns);
    if (this.inspected && (this.mode === "archive-array" || this.mode === "detail-extracted")) {
      this.scene.focusTrack(this.inspected.column.columnIndex, this.inspected.index, this.mode === "detail-extracted");
    }
    this.blockSelections = false;
  }

  private onSceneSelection(selection: SceneSelection): void {
    if (this.blockSelections || !["archive-array", "detail-extracted"].includes(this.mode)) return;
    const candidate = this.locatePosition(selection) ?? this.locate(selection.playlistId, selection.trackIndex);
    if (!candidate) return;
    this.updateInspected(candidate);
    this.setMode(selection.extracted ? "detail-extracted" : "archive-array", false);
    this.renderSelected();
    if (selection.extracted && selection.activate) void this.playTrack(candidate);
  }

  private select(target: Location, extract: boolean): void {
    this.updateInspected(target);
    this.setMode(extract ? "detail-extracted" : "archive-array", false);
    if (this.sceneReady) {
      this.blockSelections = true;
      this.scene.focusTrack(target.column.columnIndex, target.index, extract);
      if (!extract) this.scene.setViewMode(this.mode);
      this.blockSelections = false;
    }
    this.renderSelected();
  }

  private updateInspected(target: Location): void {
    const selected = this.inspected && position(this.inspected);
    const next = position(target);
    if (next.playlistId !== selected?.playlistId || next.entryId !== selected?.entryId) {
      this.leaveVideo();
      this.closeTransportMenu();
      this.videoError = "";
    }
    this.inspected = target;
  }

  private setMode(mode: SceneViewMode, updateScene = true): void {
    const next = mode === "archive-array" && this.sceneFailed ? "radar-grid" : mode;
    const changed = next !== this.mode;
    this.mode = next;
    if (changed && next !== "detail-extracted") this.closeTransportMenu();
    if (changed && next !== "detail-extracted" && this.tab === "mv-stage") {
      if (next === "disassembly-360") this.leaveVideo();
      else this.setTab("lyrics");
    }
    this.shell.dataset.mode = next;
    element("radar").hidden = next !== "radar-grid";
    element("scene-caption").hidden = next !== "archive-array" || !this.inspected;
    element("archive-navigation").hidden = next !== "archive-array" || !this.inspected;
    const detailHidden = next !== "detail-extracted" || !this.inspected;
    element("detail").hidden = detailHidden;
    element("detail-tabs").hidden = detailHidden;
    this.buttons("close-detail")[0].hidden = detailHidden;
    element("inspection-tools").hidden = next !== "disassembly-360";
    if (changed && next === "detail-extracted" && this.inspected && this.lyricTrack !== this.inspected.track.videoId && this.tab === "lyrics") {
      void this.loadLyrics(this.inspected.track.videoId);
    }
    const radar = this.buttons("radar")[0];
    radar?.setAttribute("aria-pressed", String(next === "radar-grid"));
    this.disable("radar", next === "radar-grid" && this.sceneFailed);
    if (changed && next === "radar-grid") {
      this.overviewId ??= this.inspected?.column.playlistId ?? null;
      this.renderOverview();
    }
    if (updateScene && this.sceneReady) {
      this.blockSelections = true;
      this.scene.setViewMode(next);
      this.blockSelections = false;
    }
    this.renderTransportActions();
    this.updatePower();
  }

  private toggleRadar(): void {
    if (this.sceneFailed && this.mode === "radar-grid") return;
    this.setMode(this.mode === "radar-grid" ? "archive-array" : "radar-grid");
  }

  private closeDetail(): void {
    this.setMode("archive-array");
    this.listOpened = false;
    this.setTab("lyrics");
    this.focusArchive();
  }

  private focusArchive(): void {
    if (this.sceneFailed) element("radar").querySelector("button")?.focus();
    else element("scene").querySelector<HTMLElement>("[tabindex]")?.focus();
  }

  private enterInspection(): void {
    if (!this.inspected || !this.sceneReady) return;
    this.inspectionScroll = element(this.tab).scrollTop;
    setText(element("viewer-track"), this.inspected.track.title);
    this.expandInspection(false);
    this.setMode("disassembly-360");
    this.buttons("leave-inspection")[0]?.focus();
  }

  private expandInspection(value: boolean): void {
    if (value === this.exploded) return;
    this.exploded = value;
    this.scene.setDisassembled(value);
    this.pressed("explode", value);
    this.pressed("assemble", !value);
  }

  private leaveInspection(): void {
    this.expandInspection(false);
    this.setMode("detail-extracted");
    element(this.tab).scrollTop = this.inspectionScroll;
    this.lyrics.holdScroll();
    this.buttons("transport-more")[0]?.focus({ preventScroll: true });
  }

  private renderSelected(): void {
    const location = this.inspected;
    if (!location) return;
    const { column, track } = location;
    this.renderArchiveCaption();
    const container = element("detail-tracks");
    if (this.detailColumn !== column) {
      if (this.detailColumn?.playlistId !== column.playlistId) this.listOpened = false;
      this.detailColumn = column;
      container.replaceChildren(...column.tracks.map((item, i) => makeTrackRow(item, column, i)));
      if (!column.tracks.length) container.append(empty(t("该歌单暂无曲目", "This playlist is empty.")));
    }
    this.mvPoster.src = safeAssetUrl(track.thumbnailWideUrl ?? track.thumbnailSquareUrl);
    if (this.mode === "detail-extracted" && this.tab === "lyrics" && this.lyricTrack !== track.videoId) void this.loadLyrics(track.videoId);
    this.renderPlayback();
  }

  private renderArchiveCaption(): void {
    if (!this.inspected) return;
    const { column, index, track } = this.inspected;
    const ordinal = this.columns.findIndex(item => item.playlistId === column.playlistId);
    setText(element("scene-count"), String(ordinal + 1).padStart(2, "0"));
    setText(element("scene-total"), " / " + String(this.columns.length).padStart(2, "0"));
    setText(element("scene-column"), column.title);
    setText(element("scene-owner"), column.playlistId.startsWith("archive:") ? "unfoundsound" : "Nasti");
    const scale = element<HTMLInputElement>("archive-scale");
    scale.max = String(column.tracks.length - 1);
    scale.value = String(index);
    scale.disabled = column.tracks.length <= 1;
    scale.setAttribute("aria-valuetext", (index + 1) + " / " + column.tracks.length + " · " + track.title);
    const markers = element("archive-tick-markers");
    const last = column.tracks.length - 1;
    const intervals = Math.min(last, maxArchiveTicks - 1);
    const positions = Array.from({ length: intervals + 1 }, (_, step) => intervals ? Math.round(step * last / intervals) : 0);
    if (!positions.includes(index)) positions.push(index);
    if (markers.childElementCount !== positions.length) {
      markers.replaceChildren(...positions.map(() => document.createElement("span")));
    }
    Array.from(markers.children).forEach((child, slot) => {
      const marker = child as HTMLElement;
      const tick = positions[slot];
      marker.dataset.index = String(tick);
      marker.style.left = `${last ? tick * 100 / last : 50}%`;
      marker.classList.add("archive-tick");
      marker.classList.toggle("is-active", tick === index);
    });
    const activeColumns = this.columns.filter(item => item.tracks.length);
    const activeIndex = activeColumns.findIndex(item => item.playlistId === column.playlistId);
    this.disable("previous-playlist", activeColumns.length < 2 || (!this.wrapLists && activeIndex === 0));
    this.disable("next-playlist", activeColumns.length < 2 || (!this.wrapLists && activeIndex === activeColumns.length - 1));
  }

  private renderOverview(keepFocus = false): void {
    const column = this.columns.find(item => item.playlistId === this.overviewId)
      ?? this.columns.find(item => item.playlistId === this.inspected?.column.playlistId) ?? this.columns[0];
    this.overviewId = column?.playlistId ?? null;
    const sidebar = element("radar-columns");
    sidebar.replaceChildren(...this.columns.map(item => makeOverviewPlaylist(item, item === column)));
    if (keepFocus) sidebar.querySelector<HTMLButtonElement>('[aria-current="true"]')?.focus();
    setText(element("radar-title"), column?.title ?? t("曲库总览", "Library overview"));
    const rows = column?.tracks.map((track, index) => makeTrackRow(track, column, index, true)) ?? [];
    this.youtube.decorate(column ?? null, rows);
    element("overview-tracks").replaceChildren(...(rows.length ? rows : [empty(column ? t("该歌单暂无曲目", "This playlist is empty.") : t("曲库暂无歌单", "No playlists in the library."))]));
    this.markTrackRows();
  }

  private markTrackRows(): void {
    const selected = this.inspected && position(this.inspected);
    for (const row of this.shell.querySelectorAll<HTMLElement>(".track-row[data-entry-id]")) {
      if (row.closest("[hidden]")) continue;
      const { playlistId, entryId } = row.dataset;
      const current = selected?.playlistId === playlistId && selected?.entryId === entryId;
      const playing = this.playback.status === "playing" && this.queue?.playlistId === playlistId && this.queue?.entryId === entryId;
      row.dataset.selected = String(current);
      row.dataset.playing = String(playing);
      row.querySelector(".track-main")?.setAttribute("aria-current", String(current));
      const control = row.querySelector<HTMLButtonElement>('[data-action="play-result"]');
      if (control) setIcon(control, playing ? "pause" : "play", (playing ? t("暂停 ", "Pause ") : t("播放 ", "Play ")) + (row.querySelector("strong")?.textContent ?? ""));
    }
  }

  private setTab(tab: DetailSubTab): void {
    const previous = this.tab;
    this.tab = tab;
    if (previous === "mv-stage" && tab !== "mv-stage") this.leaveVideo();
    element("detail-inner").dataset.tab = tab;
    for (const name of tabNames) {
      element(name).hidden = name !== tab;
      this.pressed(name, name === tab);
    }
    if (tab === "tracklist" && !this.listOpened && this.inspected && this.mode === "detail-extracted") {
      const row = element("detail-tracks").children.item(this.inspected.index) as HTMLElement | null;
      if (row) element("tracklist").scrollTop = Math.max(0, row.offsetTop - row.parentElement!.offsetTop);
      this.listOpened = true;
    }
    if (tab === "lyrics" && this.inspected && this.mode === "detail-extracted" && this.lyricTrack !== this.inspected.track.videoId) {
      void this.loadLyrics(this.inspected.track.videoId);
    }
    this.updatePower();
  }

  private async loadLyrics(videoId: string): Promise<void> {
    const request = ++this.lyricRequest;
    this.lyricTrack = videoId;
    this.lyrics.loading();
    try {
      const lines = await this.ipc.lyrics_get_synced({ videoId });
      if (request !== this.lyricRequest || this.inspected?.track.videoId !== videoId) return;
      this.lyrics.show(lines);
      if (this.playback.videoId === videoId) this.lyrics.update(this.playback.positionMs);
    } catch {
      if (request !== this.lyricRequest) return;
      this.lyrics.failure();
    }
  }

  private updatePower(): void {
    if (this.sceneReady) {
      this.scene.setSuperPerformance(this.power);
      this.scene.setPowerSavingPaused(document.hidden);
    }
    const silhouette = this.mode === "detail-extracted" && this.tab === "lyrics" && this.silhouette && !this.panel.open;
    element("companion").hidden = !silhouette;
    this.updateVideoFrame();
  }

  private async showPanel(panel: Panel): Promise<void> {
    if (!this.panel.open) this.panelTrigger = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    this.closeTransportMenu();
    this.closeVolumePanel(false);
    this.outputs.close();
    this.leaveVideo();
    for (const section of ["search", "settings"]) element(section + "-panel").hidden = section !== panel;
    this.panel.dataset.panel = panel;
    this.shell.dataset.panel = panel;
    setText(element("panel-title"), panel === "search" ? t("搜索与导入", "Search and import") : t("终端设置", "Settings"));
    if (!this.panel.open) this.panel.show();
    this.updatePower();
    if (panel === "search") element("search-input").focus();
    else {
      this.settings.measure();
      element("settings-panel").querySelector<HTMLElement>('[aria-current="true"]')?.focus({ preventScroll: true });
      await this.library.showVault();
    }
  }

  private async showVaultSettings(): Promise<void> {
    const opening = this.showPanel("settings");
    this.settings.jump("library", true);
    await opening;
  }

  private closePanel(): void {
    if (!this.panel.open) return;
    this.searchRequest++;
    clearTimeout(this.searchTimer);
    this.panel.close();
    delete this.shell.dataset.panel;
    this.panelTrigger?.focus({ preventScroll: true });
    this.panelTrigger = null;
    this.updatePower();
  }

  private changeFilter(filter: SearchFilter): Promise<void> {
    this.filter = filter;
    for (const value of searchFilters) this.pressed("filter-" + value, value === filter);
    return this.search();
  }

  private async search(knownPlaylist?: string): Promise<void> {
    const request = ++this.searchRequest;
    const query = element<HTMLInputElement>("search-input").value.trim();
    const area = element("search-results");
    const current = () => request === this.searchRequest && this.panel.open
      && this.panel.dataset.panel === "search" && !this.closed;
    this.searchTracks.clear();
    area.replaceChildren();
    if (!query) return;
    try {
      const playlist = knownPlaylist ?? playlistReference(query);
      if (playlist) {
        await this.youtube.preview(playlist, area, current);
        return;
      }
      area.replaceChildren(empty(t("正在搜索 YouTube Music", "Searching YouTube Music…")));
      const result = await this.ipc.catalog_search({ query, filter: this.filter });
      if (!current()) return;
      const rows = [
        ...result.songs.map((track, index) => this.searchRow(track, index, false)),
        ...result.videos.map((track, index) => this.searchRow(track, index, true)),
        ...result.playlists.map(playlist => makePlaylistRow(playlist, !this.columns.some(column => column.playlistId === playlist.playlistId))),
      ];
      area.replaceChildren(...(rows.length ? rows : [empty(t("未找到匹配的曲目或歌单", "No matching tracks or playlists."))]));
    } catch (error) {
      if (!current()) return;
      area.replaceChildren();
      this.message(localizeError(error), "error");
    }
  }

  private searchRow(track: TrackItem, index: number, video: boolean): HTMLElement {
    this.searchTracks.set(`${video ? "video" : "audio"}:${track.videoId}`, track);
    const row = makeTrackRow(track, null, index);
    if (video) row.dataset.mode = "video";
    return row;
  }

  private async toggleVault(): Promise<void> {
    const track = this.transportActionTrack();
    if (!track) return;
    this.disable("toggle-vault", true);
    try {
      if (track.isVaultDownloaded) {
        await this.ipc.vault_remove_track({ videoId: track.videoId });
        if (this.detachedTrack?.videoId === track.videoId) this.detachedTrack = { ...track, isVaultDownloaded: false, isVaultVideoDownloaded: false };
        this.message(t("已从储藏库移除 ", "Removed from Vault: ") + track.title, "success");
      } else {
        this.message(t("正在存入储藏库 ", "Saving to Vault: ") + track.title);
        const saved = await this.ipc.vault_download_track({ videoId: track.videoId, downloadMv: Boolean(track.mvVideoId) });
        if (this.detachedTrack?.videoId === track.videoId) this.detachedTrack = saved;
        this.message(t("已存入储藏库 ", "Saved to Vault: ") + track.title, "success");
      }
      await this.reloadCatalog();
    } finally {
      this.disable("toggle-vault", false);
    }
  }

  private enqueuePlayback(action: () => Promise<void>): Promise<void> {
    this.pendingPlaybackCount++;
    this.playbackVersion++;
    const next = this.pendingPlayback.then(() => {
      if (!this.closed) return action();
    }).finally(() => { this.pendingPlaybackCount--; });
    this.pendingPlayback = next.catch(() => {});
    return next;
  }

  private playbackAction(action: () => Promise<PlaybackSnapshot>): Promise<void> {
    return this.enqueuePlayback(async () => this.acceptPlayback(await action())).catch(error => this.message(localizeError(error), "error"));
  }

  private playTrack(target: Location, positionMs = 0): Promise<void> {
    return this.playAudio(target.track, position(target), positionMs);
  }

  private playVaultTrack(track: TrackItem): Promise<void> {
    const target = this.findByVideoId(track.videoId);
    return this.playAudio(track, target ? position(target) : null, 0);
  }

  private playAudio(track: TrackItem, location: Position | null, positionMs: number): Promise<void> {
    this.videoIntent++;
    this.cancelMediaSwitch();
    this.videoSession = null;
    this.videoError = "";
    return this.playbackAction(async () => {
      if (this.playback.mode === "video" && this.playback.status === "playing") {
        this.acceptPlayback(await this.ipc.playback_set_playing({ playing: false }));
      }
      if (this.closed) return this.playback;
      const next = await this.ipc.playback_load({ videoId: track.videoId, mode: "audio", positionMs, autoplay: true, title: track.title, artist: trackArtists(track) });
      this.videoSession = null;
      this.queue = location;
      this.detachedTrack = localizeTrack(track);
      return next;
    });
  }

  private cancelMediaSwitch(): void {
    if (!this.mediaSwitch) return;
    const switchId = this.mediaSwitch.id;
    this.mediaSwitch = null;
    void this.ipc.playback_cancel_preparation({ switchId }).catch(() => {});
  }

  private async switchMedia(
    target: Location, mode: MediaMode, positionMs: number, shouldPlay: () => boolean,
    onCommit?: (result: PreparedPlayback, superseded: boolean) => void,
  ): Promise<void> {
    this.cancelMediaSwitch();
    const request = { id: ++this.switchCount, mode, target };
    this.mediaSwitch = request;
    this.updateVideoFrame();
    const switchId = request.id;
    try {
      await this.ipc.playback_prepare({ switchId, videoId: target.track.videoId, mode, title: target.track.title, artist: trackArtists(target.track) });
      await this.enqueuePlayback(async () => {
        if (this.mediaSwitch !== request || this.closed) return;
        // Navigation can change while the native commit is seeking. Recheck before starting playback.
        const result = await this.ipc.playback_commit({ switchId, alignTime: false, positionMs, autoplay: false });
        if (this.closed) return;
        this.queue = position(target);
        onCommit?.(result, this.mediaSwitch !== request);
        const canPlay = () => this.mediaSwitch === request && !this.closed && shouldPlay();
        let next = result.playback;
        try {
          if (canPlay()) {
            next = await this.ipc.playback_set_playing({ playing: true });
            if (!canPlay()) next = await this.ipc.playback_set_playing({ playing: false });
          }
        } finally {
          if (!this.closed) this.acceptPlayback(next);
        }
      });
    } catch (error) {
      if (this.mediaSwitch === request) throw error;
    } finally {
      if (this.mediaSwitch === request) this.cancelMediaSwitch();
      this.updateVideoFrame();
    }
  }

  private async startVideo(): Promise<void> {
    const target = this.inspected;
    if (!target?.track.mvVideoId || !this.showsVideo(target) || this.videoUnavailable()
      || this.mediaSwitch || this.returningAudio || this.startingVideo) return;
    this.startingVideo = true;
    const intent = ++this.videoIntent;
    this.videoError = "";
    let audioTarget = target;
    let resumeMs = 0;
    try {
      await this.enqueuePlayback(async () => {
        if (intent !== this.videoIntent || !this.showsVideo(target)) return;
        const previous = await this.ipc.playback_get_state();
        if (previous.mode !== "audio" || !previous.videoId) return;
        const location = this.transportLocation();
        const track = this.transportTrack(location);
        const paused = await this.ipc.playback_set_playing({ playing: false });
        if (paused.mode === "audio" && paused.videoId === previous.videoId) {
          this.acceptPlayback(paused);
          if (track?.videoId === paused.videoId) audioTarget = location ?? this.detachedLocation(track);
          if (audioTarget.track.videoId === paused.videoId) resumeMs = paused.positionMs;
        }
      });
      if (this.closed || intent !== this.videoIntent || !this.showsVideo(target)) return;
      await this.switchMedia(target, "video", 0, () => intent === this.videoIntent && this.showsVideo(target),
        (_result, superseded) => {
          this.videoSession = { target, audioTarget, resumeMs, pauseOnExit: superseded || !this.showsVideo(target) };
        });
      if (this.videoSession?.target === target && (this.videoSession.pauseOnExit || !this.showsVideo(target))) this.leaveVideo();
    } catch (error) {
      if (this.closed) return;
      if (this.videoSession?.target === target) this.leaveVideo();
      this.videoError = t("MV 播放失败", "Could not play the music video.");
      this.message(localizeError(error), "error");
    } finally {
      this.startingVideo = false;
      this.updateVideoFrame();
    }
  }

  private showsVideo(target: Location): boolean {
    if (this.mode !== "detail-extracted" || this.tab !== "mv-stage" || this.panel.open || !this.inspected) return false;
    const selected = position(this.inspected);
    const requested = position(target);
    return selected.playlistId === requested.playlistId && selected.entryId === requested.entryId;
  }

  private leaveVideo(): void {
    this.videoIntent++;
    if (this.mediaSwitch?.mode === "video") this.cancelMediaSwitch();
    if (!this.videoSession) return;
    this.videoSession.pauseOnExit = true;
    void this.returnToAudio();
  }

  private async returnToAudio(): Promise<void> {
    const session = this.videoSession;
    if (!session || this.returningAudio) return;
    this.returningAudio = true;
    try {
      if (session.pauseOnExit) {
        await this.enqueuePlayback(async () => {
          if (!this.closed && this.videoSession === session && this.playback.mode === "video"
            && (this.playback.status === "playing" || this.playback.status === "loading")) {
            this.acceptPlayback(await this.ipc.playback_set_playing({ playing: false }));
          }
        });
      }
      if (this.closed || this.videoSession !== session) return;
      await this.switchMedia(session.audioTarget, "audio", session.resumeMs, () => false);
      if (this.videoSession !== session || this.playback.mode !== "audio") return;
      this.detachedTrack = session.audioTarget.track;
      this.videoSession = null;
      this.renderPlayback();
    } catch {
      this.videoError = t("MV 切换失败，纯音频暂时无法恢复", "Could not switch back to audio.");
      if (this.tab !== "mv-stage") this.message(this.videoError);
    } finally {
      this.returningAudio = false;
      this.updateVideoFrame();
    }
  }

  private togglePlaying(): Promise<void> {
    if (this.playback.mode === "video" && this.videoSession) {
      this.leaveVideo();
      return Promise.resolve();
    }
    if (this.playback.status === "error") {
      const target = this.locatePosition(this.queue) ?? this.findByVideoId(this.playback.videoId);
      if (target) return this.playTrack(target, this.playback.positionMs);
      if (this.detachedTrack && this.detachedTrack.videoId === this.playback.videoId) return this.playVaultTrack(this.detachedTrack);
    }
    if (!this.playback.videoId) {
      const target = this.inspected;
      return target ? this.playTrack(target) : Promise.resolve();
    }
    return this.enqueuePlayback(async () => {
      if (this.playback.mode === "video" && this.videoSession && (this.returningAudio || !this.showsVideo(this.videoSession.target))) return;
      this.acceptPlayback(await this.ipc.playback_set_playing({ playing: this.playback.status !== "playing" }));
    }).catch(error => this.message(localizeError(error), "error"));
  }

  private async changeTrack(direction: 1 | -1, wrap: boolean): Promise<void> {
    const current = this.locatePosition(this.queue) ?? this.findByVideoId(this.playback.videoId) ?? this.inspected;
    if (!current) return;
    const index = this.navigation.step(current.column, current.index, direction, { shuffle: this.shuffle, wrap });
    if (index === null) return;
    const target = this.locate(current.column.playlistId, index);
    if (target) await this.playTrack(target);
  }

  private renderPlaybackModes(): void {
    this.pressed("shuffle-tracks", this.shuffle);
    this.pressed("repeat-tracks", this.repeat !== "off");
    const mode = this.repeat === "one" ? t("单曲循环", "Repeat One")
      : this.repeat === "all" ? t("列表循环", "Repeat All") : t("循环已关闭", "Repeat Off");
    setIcon(this.buttons("repeat-tracks")[0], this.repeat === "one" ? "repeatOne" : "repeat", mode);
  }

  private async seekLyric(positionMs: number): Promise<void> {
    if (!this.inspected || !Number.isFinite(positionMs)) return;
    if (this.playback.videoId !== this.inspected.track.videoId) await this.playTrack(this.inspected, positionMs);
    else await this.playbackAction(() => this.ipc.playback_seek({ positionMs }));
  }

  private toggleTransportMenu(): void {
    const menu = element("transport-menu");
    if (!menu.hidden) {
      this.closeTransportMenu(true);
      return;
    }
    this.closeVolumePanel(false);
    this.outputs.close();
    menu.hidden = false;
    this.buttons("transport-more")[0].setAttribute("aria-expanded", "true");
    menu.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus({ preventScroll: true });
  }

  private closeTransportMenu(focus = false): void {
    const menu = element("transport-menu");
    if (menu.hidden) return;
    menu.hidden = true;
    const trigger = this.buttons("transport-more")[0];
    trigger.setAttribute("aria-expanded", "false");
    if (focus) trigger.focus({ preventScroll: true });
  }

  private toggleVolume(): void {
    const panel = element("volume-panel");
    if (!panel.hidden) {
      this.closeVolumePanel(true);
      return;
    }
    this.closeTransportMenu();
    this.outputs.close();
    panel.hidden = false;
    this.buttons("volume-panel")[0].setAttribute("aria-expanded", "true");
    this.volume.focus({ preventScroll: true });
  }

  private closeVolumePanel(focus: boolean): void {
    const panel = element("volume-panel");
    if (panel.hidden) return;
    panel.hidden = true;
    const trigger = this.buttons("volume-panel")[0];
    trigger.setAttribute("aria-expanded", "false");
    if (focus) {
      requestAnimationFrame(() => {
        if (panel.hidden) trigger.focus({ preventScroll: true });
      });
    }
  }

  private commitVolume(volume: number): Promise<void> {
    if (this.closed || !Number.isFinite(volume)) return Promise.resolve();
    volume = Math.max(0, Math.min(1, volume));
    if (Math.abs(volume - (this.volumeDraft ?? this.playback.volume)) < 0.0001) {
      return this.volumeCommit ?? Promise.resolve();
    }
    if (volume > 0) this.lastVolume = volume;
    this.volumeDraft = this.queuedVolume = volume;
    this.renderVolume(volume);
    return this.volumeCommit ??= this.flushVolume();
  }

  private async flushVolume(): Promise<void> {
    try {
      while (this.queuedVolume !== null && !this.closed) {
        const volume = this.queuedVolume;
        this.queuedVolume = null;
        try {
          const next = await this.ipc.playback_set_volume({ volume });
          if (this.closed) return;
          this.volumeRevision++;
          this.playback = { ...this.playback, volume: next.volume };
        } catch (error) {
          if (this.queuedVolume === null) this.message(localizeError(error), "error");
        }
      }
    } finally {
      this.volumeCommit = null;
      this.volumeDraft = null;
      if (!this.closed) this.renderVolume(this.playback.volume);
    }
  }

  private mute(): Promise<void> {
    const current = this.volumeDraft ?? this.playback.volume;
    if (current > 0) this.lastVolume = current;
    return this.commitVolume(current > 0 ? 0 : this.lastVolume);
  }

  private setRange(input: HTMLInputElement, value: number, max: number): void {
    input.style.setProperty("--range-fill", (max > 0 ? value / max * 100 : 0).toFixed(2) + "%");
  }

  private renderVolume(volume: number): void {
    const percent = String(Math.round(volume * 100));
    for (const input of this.volumeControls) {
      if (input.value !== percent) input.value = percent;
      this.setRange(input, volume, 1);
      if (input.getAttribute("aria-valuetext") !== percent + "%") input.setAttribute("aria-valuetext", percent + "%");
    }
    setText(element("volume-value"), percent + "%");
    setText(element("settings-volume-value"), percent + "%");
    const muteLabel = this.shortcuts.title("mute", volume > 0 ? t("静音", "Mute") : t("取消静音", "Unmute"));
    for (const button of this.buttons("mute")) setIcon(button, volume > 0 ? "volume" : "mute", muteLabel);
    for (const button of this.buttons("volume-panel")) {
      setIcon(button, volume > 0 ? "volume" : "mute", t("音量", "Volume"));
    }
  }

  private async poll(): Promise<void> {
    if (this.closed) return;
    const version = this.playbackVersion;
    const volumeRevision = this.volumeRevision;
    try {
      if (this.pendingPlaybackCount === 0) {
        const state = await this.ipc.playback_get_state();
        if (version === this.playbackVersion && !this.closed) {
          this.acceptPlayback(volumeRevision === this.volumeRevision ? state : { ...state, volume: this.playback.volume });
        }
      }
    } catch (error) {
      if (version === this.playbackVersion && !this.closed && this.playback.status !== "error") {
        this.playback = { ...this.playback, status: "error" };
        this.message(localizeError(error), "error");
        this.renderPlayback();
      }
    } finally {
      const fast = this.playback.status === "playing" || this.playback.status === "loading";
      if (!this.closed) this.pollTimer = window.setTimeout(() => void this.poll(), document.hidden ? 500 : fast ? 64 : 225);
    }
  }

  private acceptPlayback(next: PlaybackSnapshot): void {
    if (this.closed) return;
    const old = this.playback;
    this.playback = next;
    if (next.error && next.error !== old.error) {
      if (next.mode === "video" && this.videoSession) {
        this.videoError = t("MV 播放失败，正在恢复音频", "Music video failed to play. Restoring audio…");
        this.leaveVideo();
      } else this.message(localizeError(next.error));
    }
    this.renderPlayback();
    if (next.mode === "video" && this.videoSession && !this.returningAudio && !this.mediaSwitch
      && (next.status === "paused" || next.status === "ended")) this.leaveVideo();
    if (next.status === "ended" && old.status !== "ended" && this.pendingPlaybackCount === 0
      && !this.mediaSwitch && next.mode === "audio") {
      if (this.repeat === "one") void this.togglePlaying();
      else if (this.repeat === "all" || this.advanceAfterEnd) {
        if (this.repeat === "all" && !this.transportLocation()) void this.togglePlaying();
        else void this.changeTrack(1, this.repeat === "all");
      }
    }
  }

  private transportLocation(): Location | null {
    const videoId = this.playback.videoId;
    if (!videoId) return null;
    const queued = this.locatePosition(this.queue);
    return queued?.track.videoId === videoId ? queued : this.findByVideoId(videoId);
  }

  private transportTrack(location = this.transportLocation()): TrackItem | null {
    return location?.track
      ?? (this.detachedTrack?.videoId === this.playback.videoId ? this.detachedTrack : null);
  }

  private transportActionTrack(): TrackItem | undefined {
    return this.transportTrack() ?? (this.mode === "detail-extracted" ? this.inspected?.track : undefined);
  }

  private renderTransportActions(active = this.transportLocation(), now = this.transportTrack(active)): void {
    const source = active ?? (!now && this.mode === "detail-extracted" ? this.inspected : null);
    const track = now ?? source?.track;
    const key = track ? (source?.column.playlistId ?? "") + "/" + (source ? occurrenceId(source.track, source.index) : track.videoId) : "";
    if (this.transportActionKey !== key) {
      this.closeTransportMenu();
      this.transportActionKey = key;
    }
    element("transport-more").hidden = !track;
    this.disable("disassemble", !this.sceneReady || !source || source.track.videoId !== track?.videoId);
    setText(element("transport-vault"), track?.isVaultDownloaded ? t("从 Vault 移除", "Remove from Vault") : t("存入 Vault", "Save to Vault"));
  }

  private describeSeek(positionMs: number, durationMs: number): void {
    const label = t(durationLabel(positionMs) + " / " + durationLabel(durationMs),
      durationLabel(positionMs) + " of " + durationLabel(durationMs));
    if (this.seek.getAttribute("aria-valuetext") !== label) this.seek.setAttribute("aria-valuetext", label);
  }

  private renderPlayback(): void {
    const state = this.playback;
    const location = this.transportLocation();
    const now = this.transportTrack(location);
    element("transport-idle-mark").hidden = Boolean(now);
    element("transport-now").hidden = !now;
    this.renderTransportActions(location, now);
    const cover = element<HTMLImageElement>("now-cover");
    const coverUrl = now?.thumbnailSquareUrl && safeAssetUrl(now.thumbnailSquareUrl);
    cover.hidden = !coverUrl;
    if (coverUrl && cover.getAttribute("src") !== coverUrl) cover.src = coverUrl;
    const credits = now ? [now.album?.name, trackArtists(now)].filter(Boolean).join(" · ") : "";
    setText(element("now-artist"), credits);
    setText(element("now-title"), now?.title ?? "");
    const nowCopy = element("now-copy");
    const description = [now?.title, credits].filter(Boolean).join(" · ");
    if (nowCopy.title !== description) nowCopy.title = description;
    this.disable("play-toggle", !now && !this.inspected);
    this.disable("previous", !now && !this.inspected);
    this.disable("next", !now && !this.inspected);
    const active = state.status === "playing";
    this.shell.dataset.playing = String(active);
    this.markTrackRows();
    this.scene.setPlayingTrack(active ? this.queue : null);
    const playLabel = active ? t("暂停", "Pause") : t("播放", "Play");
    for (const button of this.buttons("play-toggle")) setIcon(button, active ? "pause" : "play", this.shortcuts.title("play-toggle", playLabel));
    const max = Math.max(0, state.durationMs);
    this.seek.max = String(max || 1000);
    this.seek.disabled = !state.videoId || !max;
    if (!this.scrubbing) {
      this.seek.value = String(Math.min(max, Math.max(0, state.positionMs)));
      setText(element("elapsed"), durationLabel(state.positionMs));
      this.setRange(this.seek, state.positionMs, max);
      this.describeSeek(state.positionMs, max);
    }
    setText(element("duration"), durationLabel(max));
    this.renderVolume(this.volumeDraft ?? state.volume);
    if (this.mode === "detail-extracted" && this.tab === "lyrics") {
      this.lyrics.update(state.videoId === this.inspected?.track.videoId ? state.positionMs : -1);
    }
    const bands = active && !document.hidden ? state.spectrum?.bars32 : undefined;
    this.spectrum.update(bands ? Float32Array.from(bands) : null);
    this.updateVideoFrame();
  }

  private videoUnavailable(): boolean {
    const track = this.inspected?.track;
    return Boolean(track?.isVaultDownloaded && !track.isVaultVideoDownloaded && !navigator.onLine);
  }

  private updateVideoFrame(): void {
    if (this.closed) return;
    const matched = this.playback.mode === "video" && this.playback.videoId === this.inspected?.track.videoId;
    const visible = this.mode === "detail-extracted" && this.tab === "mv-stage" && !this.panel.open && !document.hidden;
    const ready = matched && visible && !["error", "idle", "loading"].includes(this.playback.status);
    element("mv-overlay").hidden = ready;
    this.mvPoster.hidden = ready;
    const track = this.inspected?.track;
    const unavailable = !track?.mvVideoId || this.videoUnavailable();
    const preparing = this.startingVideo || this.mediaSwitch?.mode === "video" || (matched && this.playback.status === "loading");
    const hint = !track?.mvVideoId ? t("此曲暂无 MV", "This track has no music video.") : this.videoUnavailable() ? t("离线，MV 未下载", "Offline. Music video is unavailable.")
      : preparing ? t("正在准备 MV", "Preparing music video…") : this.returningAudio ? t("正在恢复音频", "Restoring audio…")
      : this.videoError || (track.mvTimelineAligned ? "" : t("MV 将从头播放", "The music video will start from the beginning."));
    setText(element("mv-hint"), hint);
    this.disable("start-mv", unavailable || preparing || this.returningAudio);
    const control = this.buttons("start-mv")[0];
    control.hidden = unavailable || preparing || this.returningAudio;
    setIcon(control, this.videoError ? "restart" : "play",
      this.videoError ? t("重试 MV", "Retry music video") : track?.mvTimelineAligned ? t("播放 MV", "Play music video") : t("从头播放 MV", "Play music video from the beginning"));
    const media = this.connection.inlineVideo;
    if (media) {
      if (media.parentElement !== this.mvViewport) this.mvViewport.append(media);
      media.hidden = !ready;
      return;
    }
    const rect = ready ? this.mvViewport.getBoundingClientRect() : null;
    // The native video surface is above the web view. Keep player popovers readable where they overlap it.
    const obscured = rect && ["transport-menu", "output-panel", "login-card", "account-popover", "toast-stack"].some(id => {
      const popup = element(id);
      if (popup.hidden || (popup instanceof HTMLDialogElement && !popup.open) || (id === "toast-stack" && !popup.childElementCount)) return false;
      const bounds = popup.getBoundingClientRect();
      return bounds.left < rect.right && bounds.right > rect.left && bounds.top < rect.bottom && bounds.bottom > rect.top;
    });
    if (obscured) this.mvPoster.hidden = false;
    const frame: PlaybackVideoFrame | null = rect && !obscured && rect.width > 2 && rect.height > 2
      ? { x: Math.round(rect.left), y: Math.round(rect.top), width: Math.round(rect.width), height: Math.round(rect.height) }
      : null;
    const key = JSON.stringify(frame);
    if (key === this.videoKey) return;
    this.videoKey = key;
    this.pendingVideoFrame = this.pendingVideoFrame.then(async () => {
      if (key !== this.videoKey || this.closed) return;
      try {
        await this.ipc.playback_set_video_frame({ frame });
      } catch (error) {
        this.message(localizeError(error), "error");
      }
    });
  }

  private message(text: string, level: ToastLevel = "info", openLogin = false): void {
    if (this.closed) return;
    const stack = element("toast-stack");
    const duplicate = [...stack.children].find(node => node.textContent === text && !(node as HTMLElement).dataset.closing);
    if (duplicate) return;
    const toast = document.createElement("div");
    toast.className = "glass toast";
    toast.dataset.level = level;
    toast.setAttribute("role", level === "error" || level === "warning" ? "alert" : "status");
    const glyph: Record<ToastLevel, IconName> = { info: "info", success: "check", warning: "warning", error: "error" };
    const mark = document.createElement("span");
    mark.innerHTML = icon(glyph[level]);
    const copy = openLogin ? document.createElement("button") : document.createElement("span");
    copy.textContent = text;
    if (copy instanceof HTMLButtonElement) {
      copy.type = "button";
      copy.onclick = () => { this.youtube.openLogin(); dismiss(); };
    }
    const close = actionButton("close", "dismiss-toast", t("关闭通知", "Dismiss notification"));
    delete close.dataset.action;
    const schedule = (callback: () => void, delay: number) => {
      const timer = window.setTimeout(() => { this.toastTimers.delete(timer); callback(); }, delay);
      this.toastTimers.add(timer);
      return timer;
    };
    const dismiss = () => {
      if (toast.dataset.closing) return;
      clearTimeout(timer);
      this.toastTimers.delete(timer);
      toast.dataset.closing = "true";
      schedule(() => { toast.remove(); this.updateVideoFrame(); }, this.motionReduced ? 0 : 180);
    };
    const timer = schedule(dismiss, level === "warning" || level === "error" ? 8500 : 4500);
    close.onclick = dismiss;
    toast.append(mark, copy, close);
    while (stack.childElementCount >= 4) stack.firstElementChild?.remove();
    stack.append(toast);
    this.updateVideoFrame();
  }

  private dispose(closeConnection = true): void {
    this.closed = true;
    this.cancelMediaSwitch();
    this.events.abort();
    this.glass.abort();
    this.frameResize.disconnect();
    this.scene.dispose();
    if (closeConnection) this.connection.dispose();
    clearTimeout(this.pollTimer);
    clearTimeout(this.searchTimer);
    for (const timer of this.toastTimers) clearTimeout(timer);
  }
}

function renderLayout(): void {
  document.documentElement.lang = language();
  document.title = t("Nasti · 莱茵生命声学工程终端", "Nasti · Rhine Lab Acoustic Terminal");
  mountLayout(element("app"));
  void setDesktopLanguage().catch(error => console.warn(error));
}

renderLayout();
void openTerminal()
  .then(connection => new Terminal(connection).start())
  .catch(error => setText(element("loading-status"), localizeError(error)));
