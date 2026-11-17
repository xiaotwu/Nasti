import * as THREE from "three";
import type { ThemeVariant, TrackItem } from "../../../contracts/ipc-commands";

export const CASE_WIDTH = 5;
export const CASE_HEIGHT = 3.7;
export const CASE_DEPTH = 0.31;

const palettes = {
  "celestial-cyan": {
    case: "#9b9385", bezel: "#f0e7d7", body: "#ded5c8", trim: "#9d9485",
    surface: "#f2eadf", surfaceEnd: "#d8cfc2", ink: "#292922", dim: "#504c43",
    grid: "#b7aa99", gridSoft: "#cfc3b3", fog: "#eae5df", glass: "#f6eee4",
  },
  "banshee-obsidian": {
    case: "#252525", bezel: "#484747", body: "#363739", trim: "#282828",
    surface: "#343433", surfaceEnd: "#202020", ink: "#efebe3", dim: "#d2cbc1",
    grid: "#4b4640", gridSoft: "#393733", fog: "#222120", glass: "#cdd1d1",
  },
} as const satisfies Record<ThemeVariant, Record<string, string>>;

export const cassettePalette = (theme: ThemeVariant) => palettes[theme];

export function faceKey(track: TrackItem) {
  return JSON.stringify([track.title, track.artists.map(artist => artist.name), track.album?.name, track.thumbnailSquareUrl]);
}

export type ArtworkContext = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

export function artworkCanvas(width: number, height: number) {
  if (typeof OffscreenCanvas !== "undefined") {
    const canvas = new OffscreenCanvas(width, height);
    const context = canvas.getContext("2d");
    if (context) return { canvas, context };
  }
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("Cannot create cassette lettering canvas");
  return { canvas, context };
}

export function artworkTexture(canvas: HTMLCanvasElement | OffscreenCanvas) {
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.minFilter = THREE.LinearFilter;
  texture.magFilter = THREE.LinearFilter;
  texture.generateMipmaps = false;
  return texture;
}

const FONT = 'system-ui, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif';

function fitted(context: ArtworkContext, text: string, width: number) {
  if (context.measureText(text).width <= width) return text;
  const characters = Array.from(text);
  let low = 0;
  let high = characters.length - 1;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    if (context.measureText(characters.slice(0, middle).join("") + "…").width <= width) low = middle;
    else high = middle - 1;
  }
  return characters.slice(0, low).join("") + "…";
}

export function paintArchiveLabel(
  ctx: ArtworkContext, x: number, y: number, width: number, height: number,
  theme: ThemeVariant, track?: TrackItem, raised = false, cover?: HTMLImageElement,
) {
  ctx.clearRect(x, y, width, height);
  if (!track) return;
  ctx.save();
  ctx.translate(x, y);
  const palette = cassettePalette(theme);
  const artist = track.artists.map(item => item.name).join(" / ");
  const left = raised && cover ? 112 : 8;
  if (raised && cover) {
    const pictureHeight = cover.naturalWidth * .48;
    ctx.drawImage(cover, 0, (cover.naturalHeight - pictureHeight) / 2, cover.naturalWidth, pictureHeight, 8, 13, 92, height - 26);
  }
  ctx.textBaseline = "middle";
  ctx.fillStyle = raised ? palette.ink : palette.dim;
  ctx.font = (raised ? "650 33px " : "550 32px ") + FONT;
  const title = raised ? track.title : [track.title, artist].filter(Boolean).join(" - ");
  ctx.fillText(fitted(ctx, title, width - left - 7), left, height * (raised ? .34 : .5));
  if (raised) {
    ctx.fillStyle = palette.dim;
    ctx.font = "520 25px " + FONT;
    const subtitle = [artist, track.album?.name].filter(Boolean).join(" - ");
    ctx.fillText(fitted(ctx, subtitle, width - left - 7), left, height * .70);
  }
  ctx.restore();
}

export class CoverImages {
  private readonly images = new Map<string, { result: Promise<HTMLImageElement | null>; cancel: () => void }>();

  load(source: string): Promise<HTMLImageElement | null> {
    if (!source) return Promise.resolve(null);
    const base = new URL(document.baseURI);
    let url: URL;
    try { url = new URL(source, base); }
    catch { return Promise.resolve(null); }
    const packaged = ["tauri:", "file:"].includes(url.protocol) && url.protocol === base.protocol && url.host === base.host;
    if (!packaged && !["http:", "https:", "blob:", "data:"].includes(url.protocol)) return Promise.resolve(null);
    const existing = this.images.get(url.href);
    if (existing) {
      this.images.delete(url.href);
      this.images.set(url.href, existing);
      return existing.result;
    }
    if (this.images.size >= 32) {
      const oldest = this.images.keys().next().value;
      if (oldest !== undefined) {
        this.images.get(oldest)?.cancel();
        this.images.delete(oldest);
      }
    }
    const image = new Image();
    if ((url.protocol === "https:" || url.protocol === "http:") && url.origin !== base.origin) {
      image.crossOrigin = "anonymous";
    }
    let cancel = () => {};
    const result = new Promise<HTMLImageElement | null>((resolve) => {
      image.onload = () => { cancel = () => {}; resolve(image); };
      image.onerror = () => { cancel = () => {}; resolve(null); };
      cancel = () => {
        image.onload = null;
        image.onerror = null;
        image.removeAttribute("src");
        resolve(null);
      };
      image.src = url.href;
    });
    this.images.set(url.href, { result, cancel: () => cancel() });
    return result;
  }

  dispose() {
    for (const image of this.images.values()) image.cancel();
    this.images.clear();
  }
}
