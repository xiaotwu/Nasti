import * as THREE from "three";
import type { PlaylistColumn, ThemeVariant, TrackItem } from "../../../contracts/ipc-commands";
import type { PlaylistOccurrence } from "../../../contracts/scene-bridge";
import {
  ARCHIVE_ROWS, ARCHIVE_LABEL_INSET, ARCHIVE_LABEL_X, HOME_PRIMARY_ROWS,
  HOME_PRIMARY_SCALE, HOME_SIDE_ROWS, SELECTION_LIFT,
  archiveHeight, archiveLanePlacement, archiveRowSpacing, sameCell, trackAt, type ArchiveCell,
} from "./archive-layout";
import {
  artworkCanvas, artworkTexture, CASE_DEPTH, CASE_HEIGHT, CASE_WIDTH,
  cassettePalette, CoverImages, faceKey, paintArchiveLabel,
} from "./cassette-artwork";

const LANE_OFFSETS = [0, -1, 1, -2, 2, -3, 3] as const;
const FRONT_ROWS = 20;
const BACK_ROWS = 10;
const CAPACITY = LANE_OFFSETS.length * (ARCHIVE_ROWS + FRONT_ROWS + BACK_ROWS);
const ATLAS_SIZE = 2048;
const TILE_WIDTH = 512;
const TILE_HEIGHT = 96;
const TILES_PER_ROW = Math.floor(ATLAS_SIZE / TILE_WIDTH);
const ATLAS_CAPACITY = TILES_PER_ROW * Math.floor(ATLAS_SIZE / TILE_HEIGHT);

interface AtlasEntry {
  index: number;
}

interface VisibleSlot {
  cell: ArchiveCell;
  track?: TrackItem;
  key: string;
  x: number;
  y: number;
  z: number;
  distance: number;
  selected: boolean;
  ghost: boolean;
  playing: boolean;
  clearance: number;
  raised: boolean;
  scale: number;
  tilt: number;
}

function caseGeometry(): THREE.BufferGeometry {
  const x = CASE_WIDTH / 2 - .016;
  const y = CASE_HEIGHT / 2 - .016;
  const outline = new THREE.Shape();
  outline.moveTo(-x, -y);
  outline.lineTo(x, -y);
  outline.lineTo(x, y);
  outline.lineTo(-x + .20, y);
  outline.quadraticCurveTo(-x, y, -x, y - .20);
  outline.closePath();
  const hole = new THREE.Path();
  hole.absarc(x - .23, y - .22, .092, 0, Math.PI * 2, true);
  outline.holes.push(hole);
  return new THREE.ExtrudeGeometry(outline, {
    depth: CASE_DEPTH - .032, bevelEnabled: true, bevelThickness: .016,
    bevelSize: .016, bevelSegments: 1, curveSegments: 7, steps: 1,
  }).translate(0, 0, -(CASE_DEPTH - .032) / 2);
}

class FaceAtlas {
  private readonly artwork = artworkCanvas(ATLAS_SIZE, ATLAS_SIZE);
  readonly texture = artworkTexture(this.artwork.canvas);
  private readonly entries = new Map<string, AtlasEntry>();
  private readonly available = Array.from({ length: ATLAS_CAPACITY - 1 }, (_, index) => ATLAS_CAPACITY - 1 - index);
  private disposed = false;

  constructor(
    private readonly covers: CoverImages,
    private theme: ThemeVariant,
    private readonly invalidate: () => void,
  ) {
    this.draw(0);
  }

  private draw(index: number, track?: TrackItem, raised = false, cover?: HTMLImageElement) {
    paintArchiveLabel(this.artwork.context, index % TILES_PER_ROW * TILE_WIDTH,
      Math.floor(index / TILES_PER_ROW) * TILE_HEIGHT, TILE_WIDTH, TILE_HEIGHT, this.theme, track, raised, cover);
    this.texture.needsUpdate = true;
  }

  setTheme(theme: ThemeVariant) {
    if (this.theme === theme) return;
    this.theme = theme;
    for (const entry of this.entries.values()) this.available.push(entry.index);
    this.entries.clear();
    this.draw(0);
  }

  bind(ordered: readonly VisibleSlot[]) {
    const needed = new Map<string, VisibleSlot>();
    for (const slot of ordered) {
      if (!slot.track || !slot.key || needed.has(slot.key)) continue;
      if (needed.size === ATLAS_CAPACITY - 1) break;
      needed.set(slot.key, slot);
    }
    for (const [key, entry] of this.entries) {
      if (needed.has(key)) continue;
      this.entries.delete(key);
      this.available.push(entry.index);
    }
    for (const [key, { track, raised }] of needed) {
      if (!track) continue;
      if (this.entries.has(key)) continue;
      const index = this.available.pop();
      if (index === undefined) break;
      const entry = { index };
      this.entries.set(key, entry);
      this.draw(index, track, raised);
      if (raised) void this.covers.load(track.thumbnailSquareUrl).then((cover) => {
        if (!this.disposed && cover && this.entries.get(key) === entry) {
          this.draw(index, track, raised, cover);
          this.invalidate();
        }
      });
    }
  }

  writeRegion(key: string, attribute: THREE.InstancedBufferAttribute, index: number) {
    const tile = this.entries.get(key)?.index ?? 0;
    const x = tile % TILES_PER_ROW * TILE_WIDTH;
    const y = Math.floor(tile / TILES_PER_ROW) * TILE_HEIGHT;
    attribute.setXYZW(index, (x + 1) / ATLAS_SIZE, 1 - (y + TILE_HEIGHT - 1) / ATLAS_SIZE,
      (TILE_WIDTH - 2) / ATLAS_SIZE, (TILE_HEIGHT - 2) / ATLAS_SIZE);
  }

  dispose() {
    this.disposed = true;
    this.entries.clear();
    this.texture.dispose();
  }
}

export interface ArchiveRenderFrame {
  columns: PlaylistColumn[];
  pan: ArchiveCell;
  selected: ArchiveCell;
  hover: ArchiveCell | null;
  studyCardVisible: boolean;
  armLift: number;
  time: number;
  idle: number;
  dt: number;
  reveal: number;
  wrapPlaylists: boolean;
  wrapTracks: boolean;
  playing: PlaylistOccurrence | null;
  homeWeight: number;
}

export class ArchiveRenderer {
  readonly root = new THREE.Group();
  private readonly shell = new THREE.MeshStandardMaterial({ color: 0xffffff, roughness: 0.52, metalness: 0.14 });
  private readonly inlay = new THREE.MeshBasicMaterial({ color: 0xffffff, toneMapped: false, transparent: true, depthWrite: false });
  private readonly atlas: FaceAtlas;
  private readonly face: THREE.MeshBasicMaterial;
  private readonly body: THREE.InstancedMesh<THREE.BufferGeometry, THREE.MeshStandardMaterial>;
  private readonly lightguide: THREE.InstancedMesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>;
  private readonly labels: THREE.InstancedMesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>;
  private readonly regions = new THREE.InstancedBufferAttribute(new Float32Array(CAPACITY * 4), 4)
    .setUsage(THREE.DynamicDrawUsage);
  private readonly grid = new THREE.GridHelper(156, 120, 0xffffff, 0xffffff);
  private readonly matrix = new THREE.Matrix4();
  private readonly scaleVector = new THREE.Vector3();
  private readonly frustum = new THREE.Frustum();
  private readonly extent = new THREE.Sphere(new THREE.Vector3(),
    Math.hypot(CASE_WIDTH, CASE_HEIGHT, CASE_DEPTH) * HOME_PRIMARY_SCALE / 2);
  private readonly tint = new THREE.Color();
  private readonly raycaster = new THREE.Raycaster();
  private readonly rayPointer = new THREE.Vector2();
  private readonly hits: THREE.Intersection[] = [];
  private readonly visible: VisibleSlot[] = [];
  private sorted: VisibleSlot[] = [];
  private keys = new WeakMap<TrackItem, string>();
  private readonly hoverLifts = new Map<string, number>();

  constructor(covers: CoverImages, private theme: ThemeVariant, invalidate: () => void) {
    this.atlas = new FaceAtlas(covers, theme, invalidate);
    this.face = new THREE.MeshBasicMaterial({ map: this.atlas.texture, toneMapped: false, alphaTest: .24, transparent: true, depthWrite: false });
    this.face.onBeforeCompile = (shader) => {
      shader.vertexShader = shader.vertexShader
        .replace("#include <common>", "#include <common>\nattribute vec4 atlasRegion;")
        .replace("#include <uv_vertex>", "#include <uv_vertex>\n#ifdef USE_MAP\n vMapUv = vMapUv * atlasRegion.zw + atlasRegion.xy;\n#endif");
    };
    this.body = new THREE.InstancedMesh(caseGeometry(),
      this.shell, CAPACITY);
    this.body.boundingSphere = new THREE.Sphere(new THREE.Vector3(), 65);
    this.lightguide = new THREE.InstancedMesh(new THREE.PlaneGeometry(.105, .20), this.inlay, CAPACITY);
    const panel = new THREE.PlaneGeometry(3.93, .54);
    panel.setAttribute("atlasRegion", this.regions);
    this.labels = new THREE.InstancedMesh(panel, this.face, CAPACITY);
    for (const mesh of [this.body, this.lightguide, this.labels]) {
      mesh.count = 0;
      mesh.frustumCulled = false;
      mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
      this.root.add(mesh);
    }
    this.grid.position.y = -4.2;
    this.grid.material.transparent = true;
    this.grid.material.opacity = 0.16;
    this.grid.material.depthWrite = false;
    this.root.add(this.grid);
    this.setTheme(theme);
  }

  setTheme(theme: ThemeVariant) {
    this.theme = theme;
    const palette = cassettePalette(theme);
    this.grid.material.color.set(palette.grid);
    this.atlas.setTheme(theme);
  }

  setQuality(floorGrid: boolean, anisotropy: number) {
    this.grid.visible = floorGrid;
    this.atlas.texture.anisotropy = anisotropy;
    this.atlas.texture.needsUpdate = true;
  }

  updateColumns() {
    this.keys = new WeakMap();
    this.hoverLifts.clear();
  }

  update(camera: THREE.PerspectiveCamera, frame: ArchiveRenderFrame) {
    const { columns, pan, selected, hover, time, idle, dt } = frame;
    this.visible.length = 0;
    const palette = cassettePalette(this.theme);
    this.frustum.setFromProjectionMatrix(this.matrix.multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse));

    const hoverKey = hover ? `${hover.lane}:${hover.row}` : null;
    if (hoverKey && !this.hoverLifts.has(hoverKey)) this.hoverLifts.set(hoverKey, 0);
    for (const [key, lift] of this.hoverLifts) {
      const target = key === hoverKey ? 0.14 : 0;
      const next = THREE.MathUtils.lerp(lift, target, 1 - Math.exp(-16 * dt));
      if (!target && next < 0.002) this.hoverLifts.delete(key);
      else this.hoverLifts.set(key, next);
    }

    if (columns.length > 0) {
      const mainLane = Math.round(pan.lane);
      const centerRow = Math.round(pan.row);
      const rowSpacing = archiveRowSpacing(frame.homeWeight);
      const inView = new Set<string>();
      const field = (cell: ArchiveCell) => archiveHeight(cell, selected, time, idle, frame.homeWeight);
      const heightAt = (cell: ArchiveCell) => {
        const lift = sameCell(cell, selected) ? SELECTION_LIFT + frame.armLift
          : cell.lane === selected.lane ? this.hoverLifts.get(`${cell.lane}:${cell.row}`) ?? 0 : 0;
        return field(cell) + lift - frame.reveal;
      };
      for (const offset of LANE_OFFSETS) {
        if (Math.abs(offset) > 2 && frame.homeWeight === 0) continue;
        const lane = mainLane + offset;
        const { x, scale, lowered } = archiveLanePlacement(lane, pan.lane, frame.homeWeight);
        const column = trackAt(columns, { lane, row: 0 })?.column;
        const decorativeLane = Math.abs(offset) >= 2 || !column || inView.has(column.playlistId)
          || (!frame.wrapPlaylists && (lane < 0 || lane >= columns.length));
        if (!decorativeLane && column) inView.add(column.playlistId);
        const homeRows = offset === 0 ? HOME_PRIMARY_ROWS : HOME_SIDE_ROWS;
        const rows = Math.round(THREE.MathUtils.lerp(ARCHIVE_ROWS, homeRows, frame.homeWeight));
        const startRow = centerRow - Math.floor(rows / 2);
        const endRow = startRow + rows;
        const backRows = Math.round(BACK_ROWS * frame.homeWeight * (decorativeLane ? 1 : .5));
        // Keep the playable rows centered and continue their spacing at the viewport edges.
        for (let row = startRow - backRows; row < endRow + FRONT_ROWS; row++) {
          const ghost = decorativeLane || row < startRow || row >= endRow;
          const cell = { lane, row };
          const active = sameCell(cell, selected);
          if (active && frame.studyCardVisible) continue;
          const z = (row - pan.row) * rowSpacing;
          const y = heightAt(cell) - lowered;
          this.extent.center.set(x, y, z);
          if (!this.frustum.intersectsSphere(this.extent)) continue;
          const item = ghost || (!frame.wrapTracks && column && (row < 0 || row >= column.tracks.length)) ? null : trackAt(columns, cell);
          const titled = item && lane === selected.lane;
          let key = titled ? this.keys.get(item.track) : undefined;
          if (titled && key === undefined) {
            key = faceKey(item.track);
            this.keys.set(item.track, key);
          }
          const raised = active || sameCell(cell, hover);
          const rise = rowSpacing * (camera.position.y - y - CASE_HEIGHT * scale / 2) / Math.max(1, camera.position.z - z);
          const clearance = THREE.MathUtils.clamp(y + lowered - heightAt({ lane, row: row + 1 }) + rise, .17, .98);
          const tilt = !active && frame.homeWeight > 0
            ? (field({ lane, row: row + .5 }) - field({ lane, row: row - .5 })) * .024 * frame.homeWeight : 0;
          const playing = item !== null && frame.playing?.playlistId === item.column.playlistId && frame.playing?.entryId === item.entryId;
          const distance = Math.abs(lane - pan.lane) * 3.8 + Math.abs(row - pan.row) * 0.64;
          this.visible.push({ cell, track: item?.track, key: key ? key + (raised ? "/raised" : "/flat") : "",
            x, y, z, distance, selected: active, ghost, clearance, raised, playing, scale, tilt });
        }
      }
    }

    this.sorted = this.visible.slice().sort((first, second) => first.distance - second.distance);
    const foreground = THREE.MathUtils.smoothstep(frame.homeWeight, .81, 1);
    this.face.opacity = foreground;
    this.inlay.opacity = foreground;
    this.labels.visible = foreground > .24;
    this.lightguide.visible = foreground > 0;
    if (this.labels.visible) this.atlas.bind(this.sorted);
    const ghostColor = new THREE.Color(palette.fog);
    const playingSlot = this.sorted.find(slot => slot.playing);
    let marked = 0;
    let labelled = 0;
    for (let index = 0; index < this.visible.length; index++) {
      const { x, y, z, key, track, selected: active, ghost, clearance, raised, playing, scale, tilt } = this.visible[index];
      this.matrix.makeRotationX(tilt).scale(this.scaleVector.setScalar(scale));
      this.matrix.setPosition(x, y, z);
      this.body.setMatrixAt(index, this.matrix);
      const color = this.tint.set(active ? palette.bezel : palette.body);
      color.lerp(ghostColor, THREE.MathUtils.lerp(.58, ghost ? .27 : 0, frame.homeWeight));
      this.body.setColorAt(index, color);
      const faceY = (CASE_HEIGHT * scale - clearance) / 2;
      const faceZ = CASE_DEPTH * scale / 2 + ARCHIVE_LABEL_INSET;
      const cosine = Math.cos(tilt), sine = Math.sin(tilt);
      const labelY = y + faceY * cosine - faceZ * sine;
      const labelZ = z + faceY * sine + faceZ * cosine;
      if (track && active) this.mark(marked++, x - 2.17 * scale, labelY, labelZ, tilt, "#c6a06a");
      if (playing && this.visible[index] === playingSlot) this.mark(marked++, x + 1.95 * scale, labelY, labelZ, tilt, "#48ad7b");
      if (key && this.labels.visible) {
        this.matrix.makeRotationX(tilt).scale(this.scaleVector.set(scale, Math.min((raised ? 1.06 : .54) * scale, clearance / .60), 1));
        this.matrix.setPosition(x + ARCHIVE_LABEL_X * scale, labelY, labelZ);
        this.labels.setMatrixAt(labelled, this.matrix);
        this.atlas.writeRegion(key, this.regions, labelled++);
      }
    }
    this.body.count = this.visible.length;
    this.lightguide.count = marked;
    this.labels.count = labelled;
    for (const mesh of [this.body, this.lightguide, this.labels]) {
      mesh.instanceMatrix.needsUpdate = true;
      if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
    }
    this.regions.needsUpdate = true;
  }

  private mark(index: number, x: number, y: number, z: number, tilt: number, color: string) {
    this.matrix.makeRotationX(tilt).setPosition(x, y, z + .002);
    this.lightguide.setMatrixAt(index, this.matrix);
    this.lightguide.setColorAt(index, this.tint.set(color));
  }

  pick(camera: THREE.Camera, rect: DOMRect, clientX: number, clientY: number): ArchiveCell | null {
    if (rect.width === 0 || rect.height === 0 || !this.root.visible) return null;
    this.rayPointer.set(2 * (clientX - rect.left) / rect.width - 1, 1 - 2 * (clientY - rect.top) / rect.height);
    this.raycaster.setFromCamera(this.rayPointer, camera);
    this.hits.length = 0;
    this.raycaster.intersectObject(this.body, false, this.hits);
    const index = this.hits[0]?.instanceId;
    const slot = index === undefined ? undefined : this.visible[index];
    return slot?.track ? slot.cell : null;
  }

  dispose() {
    for (const mesh of [this.body, this.lightguide, this.labels]) {
      mesh.geometry.dispose();
      mesh.material.dispose();
      mesh.dispose();
    }
    this.grid.geometry.dispose();
    this.grid.material.dispose();
    this.atlas.dispose();
    this.root.clear();
  }
}
