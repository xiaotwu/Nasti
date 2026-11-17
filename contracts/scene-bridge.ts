/**
 * NASTI // 2D Prism Glass UI <-> 3D Skyframe Scene & E2 Atelier Bridge
 * Decouples Agent-3 (`skyframe-3d`) and Agent-4 (`prism-glass-ui`).
 */

import type { PlaylistColumn, TrackItem, ThemeVariant } from "./ipc-commands";

export type SceneViewMode =
  | "boot"              // 娜斯提精英二主题开机动画
  | "archive-array"     // 9x32 循环三维歌曲档案盒阵列 (列即歌单，盒即歌曲)
  | "radar-grid"        // 歌单和曲目总览
  | "detail-extracted"  // 左侧原档案模型，右侧播放详情
  | "disassembly-360";  // 独立六部件拆解查看器

export type DetailSubTab = "tracklist" | "lyrics" | "mv-stage";

export interface SceneSelection {
  columnIndex: number;
  trackIndex: number;
  playlistId: string;
  entryId: string;
  extracted: boolean;
  activate?: boolean; // An explicit user action opened this occurrence for playback.
}

export type PlaylistOccurrence = Pick<SceneSelection, "playlistId" | "entryId">;

export interface AudioSpectrumFrame {
  rms: number;                  // 0.0 ~ 1.0 总体能量
  subBass: number;              // 20-80Hz 驱动 3D 拆解部件微米级声学共振与立绘机械支臂震动
  midRange: number;             // 250-2000Hz 驱动磁带盒内部双环旋转角速度
  highPresence: number;         // 4000-16000Hz 驱动精英二翠绿声学骨笔 (#48f0c8) 激光辉光强度
  bars32: Float32Array;         // 32段频谱，直接驱动精英二右下角风格的「金色竖条声波铭文」
}

export interface SkyframeSceneController {
  mount(container: HTMLElement): Promise<void>;
  dispose(): void;
  setSelectionListener(listener: ((selection: SceneSelection) => void) | null): void;
  setColumns(columns: PlaylistColumn[]): void;
  focusTrack(columnIndex: number, trackIndex: number, extractImmediately?: boolean): void;
  stepArchive(playlists: number, tracks: number): void;
  setArchiveNavigation(wrapPlaylists: boolean, wrapTracks: boolean): void;
  setReducedMotion(reduced: boolean): void;
  setPlayingTrack(occurrence: PlaylistOccurrence | null): void;
  resetView(): void;
  setViewMode(mode: SceneViewMode): void;
  setTheme(theme: ThemeVariant): void;
  setDisassembled(disassembled: boolean): void;
  setPowerSavingPaused(paused: boolean): void; // 窗口隐藏时暂停 WebGL 帧提交。
}
