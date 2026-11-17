import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import type { ThemeVariant, TrackItem } from "../../../contracts/ipc-commands";
import { artworkCanvas, artworkTexture, CoverImages, type ArtworkContext } from "./cassette-artwork";

const modelUrl = "/foundry-art/desktop/archive-cassette.glb";
const centerY = 1.8052193645990924;
const coverSize = 2.8;
const coverDepth = .170;
const tagX = -1.71;
const tagY = centerY + coverSize / 2 - .495;
const offsets = [3.35, 2.40, 1.10, -.25, -1.50, -2.65] as const;
const replacedOptics = new Set(["Subsurface_Optics", "Optical_Film_Edge", "Amber_Optical_Inlay", "Optical_Film"]);
const casing = new Set(["Frosted_Polymer", "Case_Engraving", "Case_Engraving_Highlight", "Printed_Label", "Carbon_Ink", "Index_Inlay", "Moulded_Lettering"]);
const darkMaterials: Record<string, string> = {
  Frosted_Polymer: "#efeeeb", Ivory_Edges: "#8c9498", Optical_Diffuser: "#263033",
  Internal_Ceramic: "#353d40", Titanium_Fasteners: "#b1b9bb", Index_Inlay: "#c6a36b",
  Printed_Label: "#ba9758", Optical_Edges: "#7c8382", Carbon_Ink: "#c0c5bf",
  Case_Engraving: "#777c7a", Case_Engraving_Highlight: "#969a96", Moulded_Lettering: "#7a817e",
};
const lightMaterials: Record<string, string> = {
  Frosted_Polymer: "#f7f2eb", Ivory_Edges: "#f0e7df", Optical_Diffuser: "#cec6ba",
  Internal_Ceramic: "#c7beb6", Printed_Label: "#d9bc85", Case_Engraving: "#a39d93",
  Case_Engraving_Highlight: "#e2d8c9",
};

type Part = THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial>;

function triangleSubset(geometry: THREE.BufferGeometry, select: (index: number) => boolean): [number[], number[]] {
  const index = geometry.index;
  if (!index) throw new Error("The archive model requires indexed surfaces.");
  const inside: number[] = [];
  const outside: number[] = [];
  for (let i = 0; i < index.count; i += 3) {
    const a = index.getX(i), b = index.getX(i + 1), c = index.getX(i + 2);
    const total = Number(select(a)) + Number(select(b)) + Number(select(c));
    if (total === 3) inside.push(a, b, c);
    else if (total === 0) outside.push(a, b, c);
    else throw new Error("The archive model has a triangle crossing a component boundary.");
  }
  return [inside, outside];
}

function moveTag(geometry: THREE.BufferGeometry): void {
  geometry.translate(1.36, -3.04, 0).rotateZ(Math.PI / 2).translate(tagX, tagY, 0);
}

function drawing(width: number, height: number, pixels: [number, number]) {
  const { canvas, context } = artworkCanvas(...pixels);
  const texture = artworkTexture(canvas);
  const material = new THREE.MeshBasicMaterial({ map: texture, alphaTest: .05, toneMapped: false });
  const mesh = new THREE.Mesh(new THREE.PlaneGeometry(width, height), material);
  return { context, texture, mesh };
}

function engraving(x: number, height: number) {
  const paint = drawing(1.4, height, [1024, Math.round(1024 * height / 1.4)]);
  paint.mesh.rotation.z = Math.PI / 2;
  paint.mesh.position.set(x, centerY - .7, .169);
  return paint;
}

function makeRecord() {
  const profile = [[.075, .131], [.985, .131], [.995, .136], [.995, .142], [.989, .147], [.978, .150]];
  for (let r = .969; r >= .335; r -= .012) profile.push([r, .150], [r - .0035, .1489], [r - .007, .150]);
  profile.push([.324, .150], [.317, .149], [.105, .149], [.078, .146], [.075, .142]);
  const count = 384;
  const positions: number[] = [], uv: number[] = [], triangles: number[] = [];
  const stride = profile.length;
  for (let j = 0; j < count; j++) {
    const angle = j * Math.PI * 2 / count;
    for (let k = 0; k < stride; k++) {
      const [r, z] = profile[k];
      const x = r * Math.cos(angle), y = r * Math.sin(angle);
      positions.push(x, y, z);
      uv.push((x + 1) / 2, (y + 1) / 2);
      const a = j * stride + k, b = ((j + 1) % count) * stride + k;
      const c = ((j + 1) % count) * stride + (k + 1) % stride, d = j * stride + (k + 1) % stride;
      triangles.push(a, b, d, b, c, d);
    }
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("uv", new THREE.Float32BufferAttribute(uv, 2));
  geometry.setIndex(triangles);
  geometry.computeVertexNormals();
  const { canvas, context: ctx } = artworkCanvas(1024, 1024);
  ctx.fillStyle = "#192022";
  ctx.fillRect(0, 0, 1024, 1024);
  for (let r = .35, i = 0; r < .975; r += .023, i++) {
    ctx.beginPath();
    ctx.arc(512, 512, r * 512, 0, Math.PI * 2);
    ctx.strokeStyle = i % 5 === 0 ? "#788082" : "#4b5558";
    ctx.lineWidth = i % 5 === 0 ? 2.5 : 1.8;
    ctx.stroke();
  }
  const material = new THREE.MeshPhysicalMaterial({
    color: "#d3d6d5", map: artworkTexture(canvas), roughness: .32, metalness: .17,
    clearcoat: .28, clearcoatRoughness: .23, envMapIntensity: .6,
  });
  const record = new THREE.Mesh(geometry, material);
  record.position.set(-.425, centerY, 0);
  record.name = "Grooved vinyl";
  return record;
}

export class CassetteStudy {
  readonly root = new THREE.Group();
  private readonly groups = offsets.map(() => new THREE.Group());
  private readonly cover = drawing(coverSize, coverSize, [768, 768]);
  private readonly title = engraving(-1.86, .22);
  private readonly subtitle = engraving(-1.57, .12);
  private readonly materials = new Map<Part, { surface: string; color: THREE.Color }>();
  private readonly materialsToDispose = new Set<THREE.Material>();
  private readonly geometries = new Set<THREE.BufferGeometry>();
  private readonly textures = new Set<THREE.Texture>();
  private track?: TrackItem;
  private disposed = false;
  private revision = 0;
  private openness = 0;
  private glass?: THREE.MeshPhysicalMaterial;

  constructor(private readonly covers: CoverImages, private theme: ThemeVariant, private readonly invalidate: () => void) {
    this.root.name = "NASTI archive assembly";
    for (const [index, group] of this.groups.entries()) {
      group.name = ["Fasteners", "Transparent cover and engraving", "Album cover", "Vinyl", "Liner", "Rear frame"][index];
      group.position.y = -1.85;
      this.root.add(group);
    }
    this.cover.mesh.name = "Square cover";
    this.cover.mesh.position.set(.6, centerY, coverDepth);
    this.add(this.cover.mesh, 2);
    this.add(this.title.mesh, 1);
    this.add(this.subtitle.mesh, 1);
    this.add(makeRecord(), 3);
  }

  async load() {
    const asset = await new GLTFLoader().loadAsync(modelUrl);
    const parts: Part[] = [];
    asset.scene.traverse(node => { if (node instanceof THREE.Mesh) parts.push(node as Part); });
    for (const part of parts) {
      this.geometries.add(part.geometry);
      this.materialsToDispose.add(part.material);
    }
    if (this.disposed) {
      this.dispose();
      return;
    }
    if (parts.length !== 17) throw new Error("The archive model has an unexpected component count.");
    for (const part of parts) {
      const surface = part.material.name.split(".")[0];
      if (replacedOptics.has(surface)) continue;
      if (surface === "Optical_Edges") {
        const positions = part.geometry.attributes.position;
        const [ring, edges] = triangleSubset(part.geometry, i =>
          positions.getX(i) >= -1.38 && positions.getX(i) <= 1.71
          && positions.getY(i) >= .84 && positions.getY(i) <= 3.04
          && positions.getZ(i) >= .019 && positions.getZ(i) <= .143);
        if (ring.length !== 7684 * 3) throw new Error("Unexpected optical ring geometry.");
        part.geometry.setIndex(edges);
      }
      if (surface === "Internal_Ceramic") {
        const [letters, body] = triangleSubset(part.geometry, i => part.geometry.attributes.position.getZ(i) > .22);
        if (letters.length !== 1928 * 3) throw new Error("Unexpected Tag geometry.");
        const tag = part.clone(false);
        tag.geometry = part.geometry.clone().setIndex(letters);
        moveTag(tag.geometry);
        tag.name = "Vertical tag lettering";
        this.add(tag, 1);
        part.geometry.setIndex(body);
      }
      if (surface === "Printed_Label" || surface === "Carbon_Ink") moveTag(part.geometry);
      if (surface === "Frosted_Polymer" && part.material instanceof THREE.MeshPhysicalMaterial) {
        this.glass = part.material;
        this.glass.thickness = .018;
      }
      this.materials.set(part, { surface, color: part.material.color.clone() });
      this.add(part, surface === "Titanium_Fasteners" ? 0 : casing.has(surface) ? 1 : surface === "Ivory_Edges" ? 5 : 4);
    }
    this.setTheme(this.theme);
  }

  setTrack(track: TrackItem) {
    if (this.track === track) return;
    this.track = track;
    const request = ++this.revision;
    this.paintEngravings();
    this.paintCover();
    void this.covers.load(track.thumbnailSquareUrl).then(image => {
      if (image && !this.disposed && request === this.revision) {
        this.paintCover(image);
        this.invalidate();
      }
    });
  }

  setTheme(theme: ThemeVariant) {
    this.theme = theme;
    const colors = theme === "banshee-obsidian" ? darkMaterials : lightMaterials;
    for (const [part, { surface, color }] of this.materials) {
      const tint = colors[surface];
      if (tint) part.material.color.set(tint);
      else part.material.color.copy(color);
    }
    if (this.glass) this.glass.attenuationColor.set(theme === "banshee-obsidian" ? "#eef1f2" : "#eee6df");
    this.paintEngravings();
  }

  setAnisotropy(value: number) {
    for (const texture of this.textures) {
      texture.anisotropy = value;
      texture.needsUpdate = true;
    }
  }

  pose(yaw: number, expanded: boolean, immediate: boolean, dt: number) {
    this.root.rotation.y = yaw;
    const target = Number(expanded);
    this.openness = immediate || Math.abs(this.openness - target) < .002
      ? target : THREE.MathUtils.damp(this.openness, target, 13, dt);
    for (const [index, group] of this.groups.entries()) group.position.z = this.openness * offsets[index];
    if (this.glass) {
      this.glass.transmission = .985 - .11 * this.openness;
      this.glass.roughness = .025 + .11 * this.openness;
    }
    return this.openness !== target;
  }

  dispose() {
    this.disposed = true;
    for (const geometry of this.geometries) geometry.dispose();
    for (const material of this.materialsToDispose) material.dispose();
    for (const texture of this.textures) texture.dispose();
    for (const group of this.groups) group.clear();
  }

  private add(mesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial | THREE.MeshBasicMaterial>, group: number) {
    this.geometries.add(mesh.geometry);
    this.materialsToDispose.add(mesh.material);
    if (mesh.material.map) this.textures.add(mesh.material.map);
    this.groups[group].add(mesh);
  }

  private paintCover(image?: HTMLImageElement) {
    const ctx = this.cover.context;
    const width = ctx.canvas.width;
    ctx.clearRect(0, 0, width, width);
    ctx.save();
    ctx.beginPath();
    const edge = 32;
    ctx.moveTo(edge, 0);
    ctx.arcTo(width, 0, width, width, edge);
    ctx.arcTo(width, width, 0, width, edge);
    ctx.arcTo(0, width, 0, 0, edge);
    ctx.arcTo(0, 0, width, 0, edge);
    ctx.clip();
    if (image) {
      const size = Math.min(image.naturalWidth, image.naturalHeight);
      ctx.drawImage(image, (image.naturalWidth - size) / 2, (image.naturalHeight - size) / 2, size, size, 0, 0, width, width);
    } else {
      ctx.fillStyle = this.theme === "banshee-obsidian" ? "#42474b" : "#b5afa6";
      ctx.fillRect(0, 0, width, width);
    }
    ctx.restore();
    this.cover.texture.needsUpdate = true;
  }

  private paintEngravings() {
    const track = this.track;
    this.paintLine(this.title.context, track?.title ?? "");
    this.paintLine(this.subtitle.context, [...(track?.artists.map(artist => artist.name) ?? []), track?.album?.name].filter(Boolean).join(" - "));
    this.title.texture.needsUpdate = true;
    this.subtitle.texture.needsUpdate = true;
  }

  private paintLine(ctx: ArtworkContext, text: string) {
    const { width, height } = ctx.canvas;
    const dark = this.theme === "banshee-obsidian";
    ctx.clearRect(0, 0, width, height);
    ctx.font = `550 ${Math.round(height * .72)}px -apple-system, "PingFang SC", sans-serif`;
    ctx.textBaseline = "middle";
    ctx.fillStyle = dark ? "#121413" : "#ded5c7";
    ctx.fillText(text, 2, height * .51 - 2, width - 8);
    ctx.fillStyle = dark ? "#c1c5bd" : "#6c675d";
    ctx.fillText(text, 0, height * .51, width - 8);
  }
}
