import type { PlaylistColumn, TrackItem } from "../../../contracts/ipc-commands";

export const ARCHIVE_COLUMNS = 9;
export const ARCHIVE_ROWS = 32;
export const COLUMN_SPACING = 5.2;
export const ROW_SPACING = 0.62;
export const EXTRACTED_LIFT = 4.05;
export const SELECTION_LIFT = 0.22;
export const HOME_PRIMARY_SCALE = 1.16;
export const HOME_PRIMARY_ROWS = 22;
export const HOME_SIDE_ROWS = 26;
export const ARCHIVE_LABEL_X = -.06;
export const ARCHIVE_LABEL_INSET = .012;

export function archiveColumnSpacing(homeWeight: number) {
  return COLUMN_SPACING + .13 * homeWeight;
}

export function archiveRowSpacing(homeWeight: number) {
  return ROW_SPACING * (1 + .18 * homeWeight);
}

export function archiveLanePlacement(lane: number, panLane: number, homeWeight: number) {
  const distance = lane - panLane;
  const magnitude = Math.abs(distance);
  const outer = Math.min(1, Math.max(0, magnitude - 1));
  const compression = .14 * outer + .19 * Math.max(0, magnitude - 2);
  const homeScale = HOME_PRIMARY_SCALE + (.92 - HOME_PRIMARY_SCALE) * Math.min(1, magnitude) - .11 * outer;
  return { x: (distance - Math.sign(distance) * compression * homeWeight) * archiveColumnSpacing(homeWeight),
    scale: 1 + (homeScale - 1) * homeWeight, lowered: .18 * outer * homeWeight };
}

export interface ArchiveCell {
  lane: number;
  row: number;
}

export interface Point {
  x: number;
  y: number;
}

export interface ArchiveTrack {
  column: PlaylistColumn;
  track: TrackItem;
  trackIndex: number;
  entryId: string;
}

export const wrap = (value: number, period: number) => ((value % period) + period) % period;

export function nearestOccurrence(index: number, center: number, period: number) {
  return index + Math.round((center - index) / period) * period;
}

export function entryId(track: TrackItem, index: number) {
  return track.entryId ?? `${track.videoId}:${index}`;
}

export function archiveColumns(columns: readonly PlaylistColumn[]) {
  const bySlot = new Map<number, PlaylistColumn>();
  for (const column of columns) {
    const index = column.columnIndex;
    if (Number.isInteger(index) && index >= 0 && index < ARCHIVE_COLUMNS && !bySlot.has(index)) {
      bySlot.set(index, column);
    }
  }
  return [...bySlot.values()].sort((first, second) => first.columnIndex - second.columnIndex);
}

export function trackAt(columns: readonly PlaylistColumn[], cell: ArchiveCell): ArchiveTrack | null {
  if (columns.length === 0) return null;
  const column = columns[wrap(cell.lane, columns.length)];
  if (column.tracks.length === 0) return null;
  const trackIndex = wrap(cell.row, column.tracks.length);
  const track = column.tracks[trackIndex];
  return { column, track, trackIndex, entryId: entryId(track, trackIndex) };
}

export function locateTrack(
  columns: readonly PlaylistColumn[],
  cell: ArchiveCell,
  columnIndex: number,
  trackIndex: number,
): ArchiveCell | null {
  if (!Number.isInteger(trackIndex) || trackIndex < 0) return null;
  const lane = columns.findIndex((column) => column.columnIndex === columnIndex);
  if (lane < 0 || trackIndex >= columns[lane].tracks.length) return null;
  return {
    lane: nearestOccurrence(lane, cell.lane, columns.length),
    row: nearestOccurrence(trackIndex, cell.row, columns[lane].tracks.length),
  };
}

export function sameCell(first: ArchiveCell, second: ArchiveCell | null) {
  return second !== null && first.lane === second.lane && first.row === second.row;
}

export function invertProjection(lane: Point, row: Point) {
  const det = lane.x * row.y - lane.y * row.x;
  const scale = Math.hypot(lane.x, lane.y) * Math.hypot(row.x, row.y);
  if (!Number.isFinite(det) || scale === 0 || Math.abs(det) <= scale * 0.001) return null;
  return (x: number, y: number): ArchiveCell => ({
    lane: (row.y * x - row.x * y) / det,
    row: (lane.x * y - lane.y * x) / det,
  });
}

// Analytic exponential decay keeps the release path independent of refresh rate.
export function coast(value: number, velocity: number, dt: number) {
  const decay = Math.exp(-3.4 * dt);
  return { position: value + velocity * (1 - decay) / 3.4, velocity: velocity * decay };
}

export function archiveHeight(cell: ArchiveCell, selected: ArchiveCell, time: number, idle: number, homeWeight: number) {
  const row = cell.row - selected.row;
  const lane = cell.lane - selected.lane;
  const homeHeight = lane === 0 ? -.33 - Math.min(.42, .068 * (Math.hypot(row, 1) - 1)) : -.82;
  let baseline = homeHeight;
  if (homeWeight !== 1 && (row !== 0 || lane !== 0)) {
    // As in RhineLabUI's settling field, the rows recede from the raised slot
    // rather than flattening into a visible wall in the foreground.
    const shoulder = Math.min(2.57, .17 * (Math.hypot(row, 1) - 1));
    const detailHeight = -.91 - shoulder - .91 * Math.min(1, Math.abs(lane));
    baseline = detailHeight + (homeHeight - detailHeight) * homeWeight;
  }
  if (idle === 0) return baseline;
  // RhineLabUI's idle field uses unwrapped cell coordinates, so selection does not restart its phase.
  const cycle = time * 2 * Math.PI;
  const breath = .075 * Math.sin(cycle / 8 + cell.row * .3 - cell.lane * .45)
    + .027 * Math.sin(cycle / 13 - cell.row * .17 + cell.lane * .3);
  return baseline + breath * idle;
}

export interface ExtractionPose {
  lift: number;
  speed: number;
  framing: number;
  yaw: number;
}

export const restingExtraction = (): ExtractionPose => ({ lift: SELECTION_LIFT, speed: 0, framing: 0, yaw: 0 });

const ease = (value: number) => {
  const u = Math.min(1, Math.max(0, value));
  return u ** 3 * (u * (u * 6 - 15) + 10);
};

function approach(value: number, target: number, rate: number, dt: number, threshold: number) {
  const next = target + (value - target) * Math.exp(-rate * dt);
  return Math.abs(next - target) <= threshold ? target : next;
}

function stepExtraction(pose: ExtractionPose, open: boolean, yaw: number, dt: number): ExtractionPose {
  const target = open ? EXTRACTED_LIFT : SELECTION_LIFT;

  // RhineLabUI holds the file and camera still while aligning it with its slot.
  // LBEILC/RhineLabUI@12cc5e4 src/scene.ts:1386–1423, src/motion.ts:100–118.
  if (!open && pose.yaw !== 0) {
    return { ...pose, speed: 0, yaw: approach(pose.yaw, 0, 7, dt, .001) };
  }

  // The same file stays on its rail. Its lift, rather than a second close
  // timer, drives the framing. Interrupting extraction cannot move the camera
  // further into detail as the file starts to descend.
  const rate = 4.2;
  const delta = pose.lift - target;
  const impulse = pose.speed + rate * delta;
  const decay = Math.exp(-rate * dt);
  let lift = target + (delta + impulse * dt) * decay;
  let speed = (pose.speed - rate * impulse * dt) * decay;
  if (Math.abs(lift - target) < .004 && Math.abs(speed) < .025) {
    lift = target;
    speed = 0;
  }
  const raised = ease((lift - .8) / 2.4);
  const framingTarget = open ? raised
    : Math.min(pose.framing, ease((lift - SELECTION_LIFT) / (EXTRACTED_LIFT - SELECTION_LIFT)));
  return { lift, speed, framing: approach(pose.framing, framingTarget, 4.8, dt, .002),
    yaw: open ? approach(pose.yaw, yaw * raised, 15, dt, .001) : 0 };
}

export function advanceExtraction(pose: ExtractionPose, open: boolean, yaw: number, elapsed: number, reduced: boolean): ExtractionPose {
  if (reduced) return open ? { lift: EXTRACTED_LIFT, speed: 0, framing: 1, yaw } : restingExtraction();
  // A frame can be late on a slow GPU. Subdivide the actual elapsed time so
  // alignment and descent still run in order without prolonging the animation.
  const duration = Math.min(elapsed, 1);
  const slices = Math.max(1, Math.ceil(duration * 60));
  for (let slice = 0; slice < slices; slice++) pose = stepExtraction(pose, open, yaw, duration / slices);
  return pose;
}

export function isArchiveResting(pose: ExtractionPose) {
  return pose.lift === SELECTION_LIFT && pose.framing === 0 && pose.yaw === 0;
}
