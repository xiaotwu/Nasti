import * as THREE from "three";
import type { PlaylistColumn, ThemeVariant } from "../../../contracts/ipc-commands";
import type {
  PlaylistOccurrence, SceneSelection, SceneViewMode, SkyframeSceneController,
} from "../../../contracts/scene-bridge";
import {
  advanceExtraction, ARCHIVE_LABEL_INSET, ARCHIVE_LABEL_X, archiveColumnSpacing, archiveColumns, archiveHeight, archiveLanePlacement,
  archiveRowSpacing, coast, entryId, EXTRACTED_LIFT, HOME_PRIMARY_SCALE, invertProjection, isArchiveResting,
  locateTrack, nearestOccurrence, restingExtraction, sameCell, SELECTION_LIFT, trackAt, wrap, type ArchiveCell,
} from "./archive-layout";
import { ArchiveRenderer } from "./archive-renderer";
import { CASE_DEPTH, CASE_HEIGHT, cassettePalette, CoverImages } from "./cassette-artwork";
import { CassetteStudy } from "./cassette-study";
import { qualitySettings, superPerformance, type SkyframeQualityPreset, type SkyframeQualitySettings } from "./quality";

export type { SkyframeQualityPreset, SkyframeQualitySettings } from "./quality";

export interface SkyframeScene extends SkyframeSceneController {
  setQualityPreset(preset: SkyframeQualityPreset, adjustments?: Partial<SkyframeQualitySettings>): void;
  setSuperPerformance(enabled: boolean): void;
}

interface Gesture {
  id: number;
  startX: number;
  startY: number;
  origin: ArchiveCell;
  projection: ReturnType<typeof invertProjection>;
  samples: Array<{ time: number; lane: number; row: number }>;
  moved: boolean;
  kind: "archive" | "detail" | "viewer";
  axis?: "lane" | "row" | "both";
  target?: ArchiveCell | null;
  route?: "pan" | "pull";
}

const origin = (): ArchiveCell => ({ lane: 0, row: 0 });
const radians = THREE.MathUtils.degToRad;
const blend = THREE.MathUtils.lerp;
const clamp = THREE.MathUtils.clamp;
const homeYaw = radians(59);
const homeElevation = radians(19);
const homeFocusX = .41;
const homeFocusY = .53;
const homeSpan = 8.15;
const homeDistance = 140;
const armedLift = .13;
const openDragDistance = 28;
const homeFaceExposure = SELECTION_LIFT + archiveHeight(origin(), origin(), 0, 0, 1)
  - archiveHeight({ lane: 0, row: 1 }, origin(), 0, 0, 1)
  + archiveRowSpacing(1) * Math.tan(homeElevation) / Math.cos(homeYaw);

function extracted(mode: SceneViewMode) {
  return mode === "detail-extracted" || mode === "disassembly-360";
}

type SceneLabels = { archive: string; detail: string; inspection: string };
const defaultLabels: SceneLabels = { archive: "歌曲档案阵列", detail: "播放详情档案", inspection: "360° 档案拆解" };

class Scene implements SkyframeScene {
  private renderer?: THREE.WebGLRenderer;
  private archive?: ArchiveRenderer;
  private study?: CassetteStudy;
  private covers?: CoverImages;
  private host?: HTMLElement;
  private events?: AbortController;
  private resizeObserver?: ResizeObserver;
  private scene = new THREE.Scene();
  private camera = new THREE.PerspectiveCamera(20, 1, 5, 300);
  private readonly fog = new THREE.Fog(0x0d1314, 34, 54);
  private readonly direction = new THREE.Vector3();
  private readonly right = new THREE.Vector3();
  private readonly up = new THREE.Vector3();
  private readonly aim = new THREE.Vector3();
  private readonly pickSample = new THREE.Vector2();
  private hovered: ArchiveCell | null = null;
  private hoverPending = false;
  private columns: PlaylistColumn[] = [];
  private libraryLoaded = false;
  private hasPlayableColumns = false;
  private selected = origin();
  private pan = origin();
  private velocity = origin();
  private readonly remembered = new Map<string, string>();
  private selectionListener: ((selection: SceneSelection) => void) | null = null;
  private lastSelection = "";
  private mode: SceneViewMode = "archive-array";
  private theme: ThemeVariant = "banshee-obsidian";
  private preset: SkyframeQualityPreset = "balanced";
  private adjustments: Partial<SkyframeQualitySettings> = {};
  private quality = qualitySettings("balanced", {});
  private super = false;
  private paused = false;
  private reduced = false;
  private wrapPlaylists = true;
  private wrapTracks = true;
  private playing: PlaylistOccurrence | null = null;
  private contextLost = false;
  private resized = true;
  private width = 1;
  private height = 1;
  private animation = 0;
  private lastFrame = 0;
  private elapsed = 0;
  private lastInteract = -3;
  private idleAmount = 0;
  private enteredAt = -1;
  private extraction = restingExtraction();
  private viewer = 0;
  private exploded = false;
  private orbit = { azimuth: radians(61), elevation: radians(15), zoom: 1 };
  private readonly frontYaw = Math.atan2(-.277, .931);
  private detailYaw = this.frontYaw;
  private armed: PlaylistOccurrence | null = null;
  private armLift = 0;
  private pointer: Gesture | null = null;
  private wheelTotal = 0;
  private wheelAt = 0;
  private wheelPlaylist = false;

  constructor(private readonly labels: SceneLabels) {}

  private get detail() { return this.extraction.framing; }

  async mount(container: HTMLElement) {
    if (this.renderer && this.host === container) return;
    const listener = this.selectionListener;
    this.dispose();
    this.selectionListener = listener;
    this.host = container;
    try {
      const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true, powerPreference: "high-performance" });
      this.renderer = renderer;
      this.scene = new THREE.Scene();
      this.scene.fog = this.fog;
      renderer.setClearColor(0x000000, 0);
      renderer.outputColorSpace = THREE.SRGBColorSpace;
      renderer.toneMapping = THREE.ACESFilmicToneMapping;
      renderer.toneMappingExposure = 1.13;
      const canvas = renderer.domElement;
      canvas.className = "skyframe-canvas";
      canvas.style.display = "block";
      canvas.style.width = "100%";
      canvas.style.height = "100%";
      canvas.style.touchAction = "none";
      canvas.style.cursor = "grab";

      this.covers = new CoverImages();
      this.archive = new ArchiveRenderer(this.covers, this.theme, this.requestFrame);
      this.study = new CassetteStudy(this.covers, this.theme, this.requestFrame);
      await this.study.load();
      if (this.host !== container || this.renderer !== renderer) return;
      this.scene.add(this.archive.root, this.study.root);
      const ambient = new THREE.HemisphereLight("#f5f1eb", "#464038", 2.0);
      const key = new THREE.DirectionalLight("#f8f1e7", 2.4);
      key.position.set(-5, 10, 7);
      const edge = new THREE.DirectionalLight("#d3d6df", 1.1);
      edge.position.set(4, 4, -9);
      this.scene.add(ambient, key, edge);
      this.setTheme(this.theme);
      this.applyQuality();
      container.appendChild(canvas);
      this.bindEvents(canvas);
      this.resizeObserver = new ResizeObserver(() => {
        this.resized = true;
        this.requestFrame();
      });
      this.resizeObserver.observe(container);
      this.updateCanvasMode();
      this.refreshTrack();
      this.requestFrame();
    } catch (error) {
      this.dispose();
      throw error;
    }
  }

  dispose() {
    this.cancelFrame();
    this.clearPointer();
    this.events?.abort();
    this.events = undefined;
    this.resizeObserver?.disconnect();
    this.resizeObserver = undefined;
    this.archive?.dispose();
    this.study?.dispose();
    this.covers?.dispose();
    this.scene.clear();
    this.renderer?.dispose();
    this.renderer?.forceContextLoss();
    this.renderer?.domElement.remove();
    this.renderer = undefined;
    this.archive = undefined;
    this.study = undefined;
    this.covers = undefined;
    this.host = undefined;
    this.contextLost = false;
    this.selectionListener = null;
    this.lastSelection = "";
    this.extraction = restingExtraction();
    this.armed = null;
    this.armLift = 0;
  }

  setSelectionListener(listener: ((selection: SceneSelection) => void) | null) {
    this.selectionListener = listener;
    this.lastSelection = "";
    this.publishSelection();
  }

  setColumns(columns: PlaylistColumn[]) {
    const prior = trackAt(this.columns, this.selected);
    this.columns = archiveColumns(columns);
    this.libraryLoaded = true;
    const activeLists = new Set(this.columns.map((column) => column.playlistId));
    for (const playlistId of this.remembered.keys()) {
      if (!activeLists.has(playlistId)) this.remembered.delete(playlistId);
    }
    this.archive?.updateColumns();
    const rememberedLane = this.columns.find((column) => column.playlistId === prior?.column.playlistId && column.tracks.length);
    const column = rememberedLane ?? this.columns.find((item) => item.tracks.length > 0);
    const interactiveChanged = this.hasPlayableColumns !== Boolean(column);
    this.hasPlayableColumns = Boolean(column);
    if (column) {
      const trackIndex = column.tracks.findIndex((track, index) => entryId(track, index) === prior?.entryId);
      const fallback = clamp(prior?.trackIndex ?? 0, 0, column.tracks.length - 1);
      const cell = this.withinNavigation(locateTrack(this.columns, this.selected, column.columnIndex, trackIndex >= 0 ? trackIndex : fallback), true);
      if (cell) {
        this.selected = cell;
        this.pan = { ...cell };
      }
    } else {
      this.selected = origin();
      this.pan = origin();
      this.hovered = null;
      this.hoverPending = false;
      if (extracted(this.mode)) this.changeMode("archive-array");
    }
    this.velocity = origin();
    this.armed = null;
    this.clearPointer();
    this.refreshTrack();
    if (interactiveChanged) this.updateCanvasMode();
    this.publishSelection();
    this.requestFrame();
  }

  focusTrack(columnIndex: number, trackIndex: number, extractImmediately = false) {
    const cell = this.withinNavigation(locateTrack(this.columns, this.selected, columnIndex, trackIndex), true);
    if (cell) {
      this.armed = null;
      this.select(cell, extractImmediately);
    }
  }

  stepArchive(playlists: number, tracks: number) {
    this.navigate(playlists, tracks);
  }

  setArchiveNavigation(wrapPlaylists: boolean, wrapTracks: boolean) {
    if (wrapPlaylists === this.wrapPlaylists && wrapTracks === this.wrapTracks) return;
    this.wrapPlaylists = wrapPlaylists;
    this.wrapTracks = wrapTracks;
    this.selected = this.withinNavigation(this.selected, true) ?? this.selected;
    this.pan = { ...this.selected };
    this.velocity = origin();
    this.armed = null;
    this.requestFrame();
  }

  setReducedMotion(reduced: boolean) {
    if (reduced === this.reduced) return;
    this.reduced = reduced;
    if (reduced) this.velocity = origin();
    this.requestFrame();
  }

  setPlayingTrack(occurrence: PlaylistOccurrence | null) {
    if (occurrence?.playlistId === this.playing?.playlistId && occurrence?.entryId === this.playing?.entryId) return;
    this.playing = occurrence;
    this.requestFrame();
  }

  resetView() {
    this.resetOrbit();
    this.requestFrame();
  }

  setViewMode(mode: SceneViewMode) {
    if (extracted(mode) && !trackAt(this.columns, this.selected)) return;
    this.changeMode(mode);
    this.publishSelection();
  }

  setTheme(theme: ThemeVariant) {
    this.theme = theme;
    this.fog.color.set(cassettePalette(theme).fog);
    this.archive?.setTheme(theme);
    this.study?.setTheme(theme);
    this.requestFrame();
  }

  setDisassembled(disassembled: boolean) {
    this.exploded = disassembled;
    this.requestFrame();
  }

  setQualityPreset(preset: SkyframeQualityPreset, adjustments: Partial<SkyframeQualitySettings> = {}) {
    this.preset = preset;
    this.adjustments = adjustments;
    this.applyQuality();
  }

  setSuperPerformance(enabled: boolean) {
    if (enabled === this.super) return;
    this.super = enabled;
    this.applyQuality();
  }

  setPowerSavingPaused(paused: boolean) {
    if (paused === this.paused) return;
    this.paused = paused;
    if (paused) this.clearPointer();
    this.updateCanvasMode();
  }

  private applyQuality() {
    this.quality = this.super ? superPerformance : qualitySettings(this.preset, this.adjustments);
    const anisotropy = Math.min(this.quality.atlasAnisotropy, this.renderer?.capabilities.getMaxAnisotropy() ?? 1);
    this.archive?.setQuality(this.quality.floorGrid, anisotropy);
    this.study?.setAnisotropy(anisotropy);
    this.resized = true;
    this.requestFrame();
  }

  private select(cell: ArchiveCell, open = false, restorePan = true, activate = false) {
    const choice = trackAt(this.columns, cell);
    if (!choice) return;
    const different = !sameCell(cell, this.selected);
    if (different) {
      this.armed = null;
      this.armLift = 0;
      this.selected = cell;
      this.lastInteract = this.elapsed;
    }
    if (restorePan) this.velocity = origin();
    this.remembered.set(choice.column.playlistId, choice.entryId);
    if (open) this.changeMode("detail-extracted");
    if (different) this.refreshTrack();
    this.publishSelection(activate);
    this.requestFrame();
  }

  private refreshTrack() {
    const selected = trackAt(this.columns, this.selected);
    const surface = this.mode === "disassembly-360" ? this.labels.inspection : this.mode === "detail-extracted" ? this.labels.detail : this.labels.archive;
    this.renderer?.domElement.setAttribute("aria-label", selected ? surface + " · " + selected.track.title : surface);
    if (selected && extracted(this.mode)) this.study?.setTrack(selected.track);
  }

  private publishSelection(activate = false) {
    if (!this.selectionListener) return;
    const current = trackAt(this.columns, this.selected);
    if (!current) {
      this.lastSelection = "";
      return;
    }
    const selection: SceneSelection = {
      columnIndex: current.column.columnIndex,
      trackIndex: current.trackIndex,
      playlistId: current.column.playlistId,
      entryId: current.entryId,
      extracted: extracted(this.mode),
    };
    const key = JSON.stringify(selection);
    if (!activate && key === this.lastSelection) return;
    this.lastSelection = key;
    this.selectionListener(activate ? { ...selection, activate: true } : selection);
  }

  private changeMode(mode: SceneViewMode) {
    if (mode === this.mode) return;
    const previous = this.mode;
    this.mode = mode;
    this.clearPointer();
    this.hovered = null;
    this.velocity = origin();
    this.armed = null;
    if (extracted(mode) && !extracted(previous)) {
      if (isArchiveResting(this.extraction)) this.extraction.lift += this.armLift;
      this.detailYaw = this.frontYaw;
    }
    this.armLift = 0;
    if (mode === "archive-array" && previous === "boot") this.enteredAt = this.elapsed;
    if (mode === "disassembly-360") {
      this.resetOrbit();
      this.exploded = false;
    }
    this.refreshTrack();
    this.updateCanvasMode();
  }

  private updateCanvasMode() {
    const canvas = this.renderer?.domElement;
    if (canvas) {
      const hidden = this.mode === "radar-grid" || this.mode === "boot";
      canvas.hidden = hidden;
      canvas.style.display = hidden ? "none" : "block";
      canvas.style.cursor = this.hasPlayableColumns ? "grab" : "default";
      canvas.tabIndex = this.hasPlayableColumns ? 0 : -1;
      if (this.hasPlayableColumns) {
        canvas.setAttribute("aria-keyshortcuts", this.mode === "disassembly-360" ? "ArrowLeft ArrowRight ArrowUp ArrowDown Home + -" : "ArrowLeft ArrowRight ArrowUp ArrowDown Enter");
      } else canvas.removeAttribute("aria-keyshortcuts");
    }
    if (this.canAnimate()) this.requestFrame();
    else this.cancelFrame();
  }

  private canAnimate() {
    return Boolean(this.renderer && this.host && !this.host.ownerDocument.hidden && !this.contextLost && !this.paused
      && this.mode !== "boot" && this.mode !== "radar-grid");
  }

  private requestFrame = () => {
    if (this.animation === 0 && this.canAnimate()) this.animation = requestAnimationFrame(this.frame);
  };

  private cancelFrame() {
    if (this.animation !== 0) cancelAnimationFrame(this.animation);
    this.animation = 0;
    this.lastFrame = 0;
  }

  private resize() {
    const host = this.host!;
    this.width = host.clientWidth;
    this.height = host.clientHeight;
    if (!this.width || !this.height) return false;
    const ratio = Math.min(devicePixelRatio || 1, this.quality.pixelRatioLimit) * this.quality.resolutionScale;
    this.renderer!.setPixelRatio(ratio);
    this.renderer!.setSize(this.width, this.height, false);
    this.camera.aspect = this.width / this.height;
    this.resized = false;
    return true;
  }

  private frame = (now: number) => {
    this.animation = 0;
    if (!this.canAnimate() || !this.renderer || !this.archive || !this.study) return;
    if (this.resized && !this.resize()) return;
    const gap = now - this.lastFrame;
    if (this.lastFrame && gap + 1.2 < 1000 / this.quality.maxFps) {
      this.requestFrame();
      return;
    }
    const frameTime = this.lastFrame ? gap / 1000 : 1 / 60;
    const dt = Math.min(.05, frameTime);
    this.lastFrame = now;
    this.elapsed += dt;
    const showDetail = extracted(this.mode);
    const aligning = !showDetail && this.extraction.yaw !== 0;
    if (!aligning) this.advancePan(dt);
    this.extraction = advanceExtraction(this.extraction, showDetail, this.detailYaw, frameTime, this.reduced);
    this.viewer = Number(this.mode === "disassembly-360");
    const hasTrack = trackAt(this.columns, this.selected) !== null;
    const resting = isArchiveResting(this.extraction);
    const showStudy = hasTrack && (showDetail || !resting);
    const homeWeight = 1 - this.detail;
    const reveal = this.reduced ? 0 : 2.6 * (1 - THREE.MathUtils.smoothstep(this.elapsed - this.enteredAt, 0, 0.78));
    const animateIdle = hasTrack && this.quality.idleMotion && this.mode === "archive-array";
    const idleTarget = Number(animateIdle && resting && !this.pointer && !this.armed && this.elapsed - this.lastInteract > 2.5);
    this.idleAmount = this.reduced || !this.quality.idleMotion
      ? 0 : this.approach(this.idleAmount, idleTarget, idleTarget ? .8 : 4, dt);
    const time = now / 1000;
    const idle = this.idleAmount * homeWeight;
    const baseY = archiveHeight(this.selected, this.selected, time, idle, homeWeight);
    const { x, scale, lowered } = archiveLanePlacement(this.selected.lane, this.pan.lane, homeWeight);
    const armTarget = this.armed ? armedLift : 0;
    this.armLift = this.approach(this.armLift, armTarget, 17, frameTime);
    const position = this.study.root.position;
    position.set(x, baseY + this.extraction.lift - lowered - reveal,
      (this.selected.row - this.pan.row) * archiveRowSpacing(homeWeight));
    position.multiplyScalar(1 - this.viewer);
    this.study.root.scale.setScalar(this.viewer ? 1 : scale);
    this.study.root.visible = showStudy;
    this.archive.root.visible = this.viewer < 0.98;
    this.placeCamera(position, archiveHeight(this.selected, this.selected, time, 0, homeWeight));

    if (this.archive.root.visible) {
      this.archive.update(this.camera, { columns: this.columns, libraryLoaded: this.libraryLoaded, pan: this.pan, selected: this.selected,
        hover: this.hovered, studyCardVisible: showStudy, armLift: this.armLift, time, idle,
        wrapPlaylists: this.wrapPlaylists, wrapTracks: this.wrapTracks, playing: this.playing,
        dt: this.reduced || !this.quality.idleMotion ? 1 : dt, reveal, homeWeight });
      this.readHover();
    }
    const expansion = showStudy && this.study.pose(this.viewer ? 0 : this.extraction.yaw,
      Boolean(this.viewer && this.exploded), this.reduced || !this.viewer, dt);
    this.renderer.render(this.scene, this.camera);
    const transitioning = showDetail ? this.detail !== 1 || this.extraction.yaw !== this.detailYaw || this.extraction.lift !== EXTRACTED_LIFT : !resting;
    if (!this.reduced && (animateIdle || this.idleAmount > 0 || reveal > 0 || transitioning || this.armLift !== armTarget
      || this.velocity.lane !== 0 || this.velocity.row !== 0 || !sameCell(this.pan, this.selected)
      || expansion)) this.requestFrame();
  };

  private approach(value: number, target: number, speed: number, dt: number) {
    return this.reduced || Math.abs(value - target) < 0.002 ? target : THREE.MathUtils.damp(value, target, speed, dt);
  }

  private placeCamera(model: THREE.Vector3, archiveY: number) {
    const yaw = blend(homeYaw, -this.frontYaw, this.detail);
    const elevation = blend(blend(homeElevation, radians(13.77), this.detail), this.orbit.elevation, this.viewer);
    const distance = blend(blend(homeDistance, 72, this.detail), 15.7, this.viewer);
    const aspect = this.camera.aspect;
    const compact = this.width < 640;
    const detailSpan = Math.max(5.9, 5.23 / (aspect * (compact ? .84 : .50)));
    const archiveSpan = homeSpan * Math.max(1, (16 / 9) / aspect);
    const span = blend(blend(archiveSpan, detailSpan, this.detail),
      Math.max(9.0, 8.7 / aspect) / this.orbit.zoom, this.viewer);
    this.direction.set(-Math.sin(yaw) * Math.cos(elevation), Math.sin(elevation), Math.cos(yaw) * Math.cos(elevation));
    if (this.viewer) this.direction.set(Math.sin(this.orbit.azimuth) * Math.cos(elevation), Math.sin(elevation), Math.cos(this.orbit.azimuth) * Math.cos(elevation));
    this.right.set(this.direction.z, 0, -this.direction.x).normalize();
    this.up.crossVectors(this.direction, this.right).normalize();
    const stageX = compact ? .5 : .262;
    const stageY = compact ? .31 : .516;
    const anchor = this.detail * (1 - this.viewer);
    // Use the resting label as the camera anchor so the selected cassette can breathe.
    this.aim.set(ARCHIVE_LABEL_X * HOME_PRIMARY_SCALE,
      archiveY + SELECTION_LIFT + (CASE_HEIGHT * HOME_PRIMARY_SCALE - homeFaceExposure) / 2,
      CASE_DEPTH * HOME_PRIMARY_SCALE / 2 + ARCHIVE_LABEL_INSET).lerp(model, this.detail);
    this.aim.addScaledVector(this.up, (homeFocusY - .5) * span * (1 - this.detail));
    this.aim.addScaledVector(this.right, (.5 - homeFocusX) * span * aspect * (1 - this.detail) * (1 - this.viewer));
    this.aim.addScaledVector(this.right, (0.5 - stageX) * span * aspect * anchor);
    this.aim.addScaledVector(this.up, (stageY - 0.5) * span * anchor);
    this.camera.near = this.viewer ? .1 : 5;
    this.camera.far = this.viewer ? 100 : 300;
    this.camera.position.copy(this.aim).addScaledVector(this.direction, distance);
    this.camera.lookAt(this.aim);
    this.camera.fov = THREE.MathUtils.radToDeg(2 * Math.atan(span / (2 * distance)));
    this.camera.updateProjectionMatrix();
    this.camera.updateMatrixWorld();
    this.fog.near = distance + blend(-.6, 0.5, this.detail);
    this.fog.far = distance + blend(13.8, 13, this.detail);
  }

  private advancePan(dt: number) {
    if (this.pointer?.kind === "archive") return;
    if (this.velocity.lane !== 0 || this.velocity.row !== 0) {
      const lane = coast(this.pan.lane, this.velocity.lane, dt);
      const row = coast(this.pan.row, this.velocity.row, dt);
      this.pan = { lane: lane.position, row: row.position };
      this.velocity = Math.hypot(lane.velocity, row.velocity) >= 0.35
        ? { lane: lane.velocity, row: row.velocity } : origin();
      this.selectNearest();
    } else {
      this.pan.lane = this.approach(this.pan.lane, this.selected.lane, 11, dt);
      this.pan.row = this.approach(this.pan.row, this.selected.row, 11, dt);
    }
  }

  private selectNearest() {
    this.pan = this.withinNavigation(this.pan) ?? this.pan;
    const cell = { lane: Math.round(this.pan.lane), row: Math.round(this.pan.row) };
    if (!sameCell(cell, this.selected)) this.select(cell, false, false);
  }

  private withinNavigation(cell: ArchiveCell | null, normalize = false): ArchiveCell | null {
    if (!cell || !this.columns.length) return cell;
    const lane = this.wrapPlaylists ? cell.lane : normalize ? wrap(cell.lane, this.columns.length) : clamp(cell.lane, 0, this.columns.length - 1);
    const count = this.columns[wrap(Math.round(lane), this.columns.length)].tracks.length;
    const row = this.wrapTracks || !count ? cell.row : normalize ? wrap(cell.row, count) : clamp(cell.row, 0, count - 1);
    return { lane, row };
  }

  private navigate(lanes: number, rows: number) {
    if (this.columns.length === 0) return;
    this.armed = null;
    this.lastInteract = this.elapsed;
    if (!lanes) {
      const candidate = this.withinNavigation({ lane: this.selected.lane, row: this.selected.row + rows });
      if (candidate) this.select(candidate);
      return;
    }
    for (let offset = 0; offset < this.columns.length; offset++) {
      const lane = this.selected.lane + lanes + Math.sign(lanes) * offset;
      if (!this.wrapPlaylists && (lane < 0 || lane >= this.columns.length)) return;
      const column = this.columns[wrap(lane, this.columns.length)];
      if (column.tracks.length === 0) continue;
      const rememberedTrack = this.remembered.get(column.playlistId);
      const restoredIndex = column.tracks.findIndex((track, index) => entryId(track, index) === rememberedTrack);
      const index = Math.max(0, restoredIndex);
      this.select({ lane, row: this.wrapTracks ? nearestOccurrence(index, this.selected.row, column.tracks.length) : index });
      return;
    }
  }

  private projectDrag() {
    const center = this.aim;
    const projection = (axis: THREE.Vector3) => {
      const start = center.clone().addScaledVector(axis, -0.5).project(this.camera);
      const end = center.clone().addScaledVector(axis, 0.5).project(this.camera);
      return { x: (end.x - start.x) * this.width / 2, y: (start.y - end.y) * this.height / 2 };
    };
    const homeWeight = 1 - this.detail;
    return invertProjection(projection(new THREE.Vector3(-archiveColumnSpacing(homeWeight), 0, 0)),
      projection(new THREE.Vector3(0, 0, -archiveRowSpacing(homeWeight))));
  }

  private resetOrbit() {
    this.orbit = { azimuth: radians(61), elevation: radians(15), zoom: 1 };
  }

  private clearPointer() {
    const gesture = this.pointer;
    this.pointer = null;
    const canvas = this.renderer?.domElement;
    if (gesture && canvas?.hasPointerCapture(gesture.id)) canvas.releasePointerCapture(gesture.id);
  }

  private canBrowse() {
    return this.hasPlayableColumns && this.mode === "archive-array" && isArchiveResting(this.extraction);
  }

  private matchesArmed(cell: ArchiveCell | null | undefined) {
    if (!cell || !this.armed || !sameCell(cell, this.selected)) return false;
    const item = trackAt(this.columns, cell);
    return this.armed.playlistId === item?.column.playlistId && this.armed.entryId === item.entryId;
  }

  private tapArchive(cell: ArchiveCell | null) {
    if (cell && this.matchesArmed(cell)) {
      this.select(cell, true, true, true);
      return;
    }
    const item = cell && trackAt(this.columns, cell);
    this.armed = null;
    if (cell && item) {
      this.select(cell);
      this.armed = { playlistId: item.column.playlistId, entryId: item.entryId };
    }
  }

  private readHover() {
    if (!this.hoverPending) return;
    this.hoverPending = false;
    if (this.pointer || !this.canBrowse() || !this.renderer || !this.archive) return;
    const canvas = this.renderer.domElement;
    const cell = this.archive.pick(this.camera, canvas.getBoundingClientRect(), this.pickSample.x, this.pickSample.y);
    if (this.hovered ? !sameCell(this.hovered, cell) : cell !== null) {
      this.hovered = cell;
      if (this.reduced || !this.quality.idleMotion) this.requestFrame();
    }
    canvas.style.cursor = this.matchesArmed(cell) ? "ns-resize" : cell ? "pointer" : "grab";
  }

  private bindEvents(canvas: HTMLCanvasElement) {
    const document = canvas.ownerDocument;
    this.events = new AbortController();
    const signal = this.events.signal;
    document.addEventListener("visibilitychange", () => {
      if (document.hidden) this.clearPointer();
      this.updateCanvasMode();
    }, { signal });
    canvas.addEventListener("webglcontextlost", (event) => {
      event.preventDefault();
      this.contextLost = true;
      this.cancelFrame();
    }, { signal });
    canvas.addEventListener("webglcontextrestored", () => {
      this.contextLost = false;
      this.resized = true;
      this.requestFrame();
    }, { signal });

    canvas.addEventListener("pointerdown", (event) => {
      if (event.button !== 0 || this.paused || this.pointer) return;
      const kind = this.mode === "disassembly-360" ? "viewer" : this.mode === "detail-extracted" ? "detail" : "archive";
      if (kind === "archive" && !this.canBrowse()) return;
      canvas.focus({ preventScroll: true });
      canvas.setPointerCapture(event.pointerId);
      this.pointer = {
        id: event.pointerId, startX: event.clientX, startY: event.clientY,
        origin: kind === "viewer" ? { lane: this.orbit.azimuth, row: this.orbit.elevation }
          : kind === "detail" ? { lane: this.detailYaw, row: 0 } : { ...this.pan },
        projection: kind === "archive" ? this.projectDrag() : null,
        target: kind === "archive" ? this.archive?.pick(this.camera, canvas.getBoundingClientRect(), event.clientX, event.clientY) : null,
        samples: [{ time: event.timeStamp, ...this.pan }], moved: false, kind,
      };
      this.velocity = origin();
      this.hovered = null;
      this.lastInteract = this.elapsed;
      this.requestFrame();
    }, { signal });
    canvas.addEventListener("pointermove", (event) => {
      const drag = this.pointer;
      if (!drag) {
        if (!this.canBrowse() || event.pointerType !== "mouse") return;
        this.lastInteract = this.elapsed;
        this.pickSample.set(event.clientX, event.clientY);
        this.hoverPending = true;
        this.requestFrame();
        return;
      }
      if (event.pointerId !== drag.id) return;
      const dx = event.clientX - drag.startX;
      const dy = event.clientY - drag.startY;
      drag.moved ||= Math.hypot(dx, dy) > 7;
      if (!drag.moved) return;
      canvas.style.cursor = "grabbing";
      this.lastInteract = this.elapsed;
      if (drag.kind === "viewer") {
        this.orbit.azimuth = drag.origin.lane - dx * 0.008;
        this.orbit.elevation = clamp(drag.origin.row + dy * 0.008, -1.46, 1.46);
      } else if (drag.kind === "detail") {
        this.detailYaw = clamp(drag.origin.lane + dx * .004, this.frontYaw - .8, this.frontYaw + .8);
      } else if (drag.projection) {
        drag.route ??= this.matchesArmed(drag.target) && -dy > Math.abs(dx) * 1.45 ? "pull" : "pan";
        if (drag.route === "pull") {
          canvas.style.cursor = "ns-resize";
          return;
        }
        this.armed = null;
        const step = drag.projection(dx, dy);
        drag.axis ??= Math.abs(dx) > 1.4 * Math.abs(dy) ? "row"
          : Math.abs(dy) > 1.4 * Math.abs(dx) ? "lane" : "both";
        this.pan = {
          lane: drag.origin.lane + (drag.axis === "row" ? 0 : step.lane),
          row: drag.origin.row + (drag.axis === "lane" ? 0 : step.row),
        };
        drag.samples.push({ time: event.timeStamp, ...this.pan });
        drag.samples = drag.samples.filter((sample) => event.timeStamp - sample.time <= 105).slice(-24);
        this.selectNearest();
      }
      this.requestFrame();
    }, { signal });
    canvas.addEventListener("pointerup", (event) => {
      const drag = this.pointer;
      if (!drag || drag.id !== event.pointerId) return;
      this.clearPointer();
      canvas.style.cursor = "grab";
      if (drag.kind !== "archive" || !this.canBrowse()) return;
      const dx = event.clientX - drag.startX;
      const dy = event.clientY - drag.startY;
      const up = -dy >= openDragDistance && -dy > Math.abs(dx) * 1.45;
      if (drag.target && drag.route !== "pan" && up && this.matchesArmed(drag.target)) {
        this.select(drag.target, true, true, true);
      } else if (!drag.moved && Math.hypot(dx, dy) <= 7) {
        this.tapArchive(drag.target ?? null);
      } else if (!this.reduced) {
        const first = drag.samples[0];
        const last = drag.samples.at(-1);
        if (first && last && last.time - first.time > 8 && event.timeStamp - last.time < 85) {
          const scale = 1000 / (last.time - first.time);
          this.velocity = { lane: clamp((last.lane - first.lane) * scale, -25, 25),
            row: clamp((last.row - first.row) * scale, -95, 95) };
        }
      }
      this.requestFrame();
    }, { signal });
    for (const type of ["pointercancel", "lostpointercapture"] as const) {
      canvas.addEventListener(type, (event) => {
        if (this.pointer?.id === event.pointerId) {
          this.clearPointer();
          canvas.style.cursor = "grab";
          this.requestFrame();
        }
      }, { signal });
    }
    canvas.addEventListener("pointerleave", () => {
      if (this.pointer || !this.hasPlayableColumns) return;
      this.hoverPending = false;
      this.hovered = null;
      canvas.style.cursor = "grab";
      this.requestFrame();
    }, { signal });
    canvas.addEventListener("wheel", (event) => {
      if (event.ctrlKey || this.paused || this.pointer) return;
      if (this.mode === "disassembly-360") {
        event.preventDefault();
        this.orbit.zoom = clamp(this.orbit.zoom * Math.exp(-event.deltaY * 0.0014), 0.58, 2);
        this.requestFrame();
      } else if (this.canBrowse()) {
        event.preventDefault();
        this.lastInteract = this.elapsed;
        const vertical = Math.abs(event.deltaY) >= Math.abs(event.deltaX);
        const playlist = vertical && !event.shiftKey;
        const delta = vertical ? event.deltaY : event.deltaX;
        if (playlist !== this.wheelPlaylist || event.timeStamp - this.wheelAt > 145 || delta * this.wheelTotal < 0) this.wheelTotal = 0;
        this.wheelPlaylist = playlist;
        this.wheelAt = event.timeStamp;
        this.wheelTotal += delta * (event.deltaMode === 0 ? 1 : 24);
        const steps = Math.trunc(this.wheelTotal / 44);
        if (steps) {
          this.wheelTotal -= steps * 44;
          this.navigate(playlist ? steps : 0, playlist ? 0 : steps);
        }
      }
    }, { signal, passive: false });

    document.addEventListener("keydown", (event) => {
      if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || this.paused) return;
      if (event.target instanceof Element && event.target.closest("button, input, textarea, select, a[href], dialog, [role=dialog], [contenteditable]")) return;
      const arrow = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[event.key];
      if (this.mode === "disassembly-360") {
        if (event.key === "Home") this.resetOrbit();
        else if (event.key === "+" || event.key === "-") this.orbit.zoom = clamp(this.orbit.zoom * (event.key === "+" ? 1.14 : .88), .58, 2);
        else if (arrow) {
          this.orbit.azimuth -= arrow[0] * radians(9);
          this.orbit.elevation = clamp(this.orbit.elevation - arrow[1] * radians(7), -1.46, 1.46);
        } else return;
        this.requestFrame();
      } else if (this.mode === "detail-extracted" && arrow && arrow[0]) {
        this.detailYaw = clamp(this.detailYaw + arrow[0] * radians(9), this.frontYaw - .8, this.frontYaw + .8);
        this.requestFrame();
      } else if (this.canBrowse()) {
        if (arrow) this.navigate(arrow[1], arrow[0]);
        else if (event.key === "Enter" && !event.repeat) this.select(this.selected, true, true, true);
        else return;
      } else return;
      event.preventDefault();
    }, { signal });
  }
}

export function createSkyframeScene(labels: SceneLabels = defaultLabels): SkyframeScene {
  return new Scene(labels);
}
