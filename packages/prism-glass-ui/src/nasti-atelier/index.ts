import { t } from "../i18n";
import tokens from "../../../../assets/foundry-art/nasti-illust/e2-visual-tokens.json";
import type { AudioSpectrumFrame } from "../../../../contracts/scene-bridge";
import { artUrl } from "../layout";

type Region = readonly (readonly [number, number])[];

// Matte coordinates are in the 1024 x 1024 reference image's unit square.
// Feathered overlaps retain the parent image under a few pixels of movement.
const mattes: Region[][] = [
  [
    [[.05,.65],[.28,.58],[.63,.58],[.90,.68],[.98,.83],[.85,.98],[.34,1],[.10,.87]],
    [[.04,.20],[.20,.20],[.20,.31],[.06,.30]],
    [[.58,.03],[.95,.09],[.96,.33],[.82,.34],[.80,.21],[.61,.21]],
  ],
  [
    [[.07,.36],[.28,.35],[.39,.40],[.45,.54],[.37,.66],[.18,.72],[.11,.58]],
    [[.53,.26],[.80,.32],[.89,.54],[.81,.64],[.67,.69],[.52,.61],[.48,.48]],
  ],
  [
    [[.38,.31],[.45,.35],[.50,.36],[.51,.30],[.59,.30],[.65,.37],[.64,.48],[.68,.56],
     [.64,.65],[.59,.68],[.60,.82],[.57,.91],[.51,.91],[.48,.86],[.47,.77],[.39,.70],
     [.38,.57],[.43,.45],[.42,.40],[.37,.36]],
    [[.22,.48],[.37,.48],[.41,.58],[.37,.70],[.27,.70],[.14,.64]],
  ],
  [
    [[.18,.17],[.23,.17],[.42,.32],[.49,.38],[.45,.41],[.33,.32],[.23,.27],[.19,.22]],
  ],
  [
    [[.68,.65],[.84,.65],[.86,.72],[.68,.72]],
    [[.27,.84],[.48,.83],[.55,.93],[.54,1],[.31,.99],[.21,.93]],
  ],
];

const vertex = `#version 300 es
precision highp float;
out vec2 vUv;
void main() {
  vec2 p=vec2(float((gl_VertexID<<1)&2),float(gl_VertexID&2));
  vUv=vec2(p.x, 1.0-p.y);
  gl_Position=vec4(p*2.0-1.0,0.0,1.0);
}`;

const fragment = `#version 300 es
precision highp float;
in vec2 vUv;
out vec4 color;
uniform sampler2D uArt;
uniform sampler2D uMattes[5];
uniform vec2 uPointer;
uniform float uClock;
uniform float uWeight[7];
uniform float uLow;
uniform float uHigh;

vec3 imageAt(vec2 p) {
  return texture(uArt, clamp(p, 0.0, 1.0)).rgb;
}
vec2 drift(int i) { return uPointer * uWeight[i] * .014; }

void main() {
  vec2 p=vUv;
  vec2 c=p-.5-drift(0);
  vec2 uv=(p-.5)*.991+.5;
  float diamond=abs(c.x)+abs(c.y);
  float edge=1.0-smoothstep(.0015,.006,abs(diamond-.478));
  float aperture=1.0-smoothstep(.466,.490,diamond);

  // L1 background stays under foreground mattes, filling their feathered edges.
  vec3 art=imageAt(uv-drift(1));
  float scan=sin((uv.y-uClock/8.0)*80.0)*.5+.5;
  art+=vec3(.012,.021,.015)*scan*(1.0-smoothstep(.1,.48,uv.y));

  vec2 cityShift=drift(2)+vec2(0.0,sin(uClock*.5712)*.0024);
  float city=texture(uMattes[0], uv).r;
  art=mix(art,imageAt(uv-cityShift),city);
  vec2 armShift=drift(3)+vec2(sin(uClock*30.0),cos(uClock*24.0))*uLow*.0018;
  float ribs=texture(uMattes[1], uv).r;
  art=mix(art,imageAt(uv-armShift),ribs);
  vec2 body=p-drift(4);
  body.x-=sin(uClock*.967+uv.y*8.0)*.0018*(1.0-smoothstep(.3,.7,uv.y));
  body.y-=sin(uClock*.967)*.0015;
  float figure=texture(uMattes[2],uv).r;
  art=mix(art,imageAt(body),figure);
  vec2 wandUv=uv-drift(5);
  float wand=texture(uMattes[3],uv).r;
  vec3 wandPixel=imageAt(wandUv);
  float green=max(0.0, wandPixel.g - max(wandPixel.r*.89,wandPixel.b*.93));
  art=mix(art,wandPixel+vec3(.04,.72,.50)*green*uHigh*1.75,wand);
  float debris=texture(uMattes[4],uv).r;
  art=mix(art,imageAt(uv-drift(6)-vec2(0.0,uLow*.0020)),debris);

  // A shallow opening is shared by all layers. Only silhouette details can leave it.
  float spill=max(city*.39,max(figure*.45,wand*.83));
  float content=smoothstep(.016,.084,max(max(art.r,art.g),art.b)) * max(aperture,spill);
  vec3 rim=vec3(.26,.74,.62);
  float rimLight=edge*(.19+.20*max(0.0,1.0-length(c-uPointer*.28)));
  color=vec4(mix(art,rim,clamp(rimLight,0.0,1.0)),max(content,rimLight));
}`;

interface RenderState {
  gl: WebGL2RenderingContext;
  program: WebGLProgram;
  textures: WebGLTexture[];
  clock: WebGLUniformLocation | null;
  pointer: WebGLUniformLocation | null;
  low: WebGLUniformLocation | null;
  high: WebGLUniformLocation | null;
}

function matte(regions: readonly Region[]): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 512;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("Canvas masks unavailable");
  context.fillStyle = "black";
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.fillStyle = "white";
  context.filter = "blur(4px)";
  for (const region of regions) {
    context.beginPath();
    region.forEach(([x, y], index) => {
      if (index === 0) context.moveTo(x * 512, y * 512);
      else context.lineTo(x * 512, y * 512);
    });
    context.closePath();
    context.fill();
  }
  return canvas;
}

function compile(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) throw new Error("WebGL shaders unavailable");
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (gl.getShaderParameter(shader, gl.COMPILE_STATUS)) return shader;
  const reason = gl.getShaderInfoLog(shader);
  gl.deleteShader(shader);
  throw new Error(reason ?? "Atelier shader compilation failed");
}

function upload(gl: WebGL2RenderingContext, index: number, image: TexImageSource): WebGLTexture {
  const texture = gl.createTexture();
  if (!texture) throw new Error("Atelier texture unavailable");
  gl.activeTexture(gl.TEXTURE0 + index);
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
  return texture;
}

export class NastiAtelier {
  private readonly canvas = document.createElement("canvas");
  private readonly fallback = new Image();
  private readonly pointer = { x: 0, y: 0 };
  private readonly resize: ResizeObserver;
  private readonly events = new AbortController();
  private state: RenderState | null = null;
  private started = false;
  private visible = false;
  private reduced = false;
  private frame: number | null = null;
  private low = 0;
  private high = 0;

  constructor(private readonly container: HTMLElement) {
    this.canvas.className = "atelier-art";
    this.canvas.setAttribute("aria-hidden", "true");
    this.canvas.hidden = true;
    this.fallback.className = "atelier-fallback";
    this.fallback.alt = t("娜斯提精英二立绘", "Nasti Elite 2 art");
    this.fallback.src = artUrl;
    this.container.append(this.fallback, this.canvas);
    const { signal } = this.events;
    this.canvas.addEventListener("webglcontextlost", event => {
      event.preventDefault();
      this.state = null;
      this.canvas.hidden = true;
      this.fallback.hidden = false;
      this.stop();
    }, { signal });
    this.canvas.addEventListener("webglcontextrestored", () => {
      this.started = false;
      if (this.visible) this.setup();
    }, { signal });
    this.container.addEventListener("pointermove", event => {
      const rect = container.getBoundingClientRect();
      this.pointer.x = (event.clientX - rect.left) / rect.width * 2 - 1;
      this.pointer.y = (event.clientY - rect.top) / rect.height * 2 - 1;
    }, { passive: true, signal });
    this.container.addEventListener("pointerleave", () => {
      this.pointer.x = this.pointer.y = 0;
    }, { passive: true, signal });
    this.resize = new ResizeObserver(() => this.schedule());
    this.resize.observe(container);
  }

  setVisibility(visible: boolean, reduced: boolean): void {
    if (this.visible === visible && this.reduced === reduced) return;
    this.visible = visible;
    this.reduced = reduced;
    if (!visible) {
      this.stop();
      return;
    }
    this.setup();
    this.schedule();
  }

  setSpectrum(frame: AudioSpectrumFrame | null): void {
    this.low = frame?.subBass ?? 0;
    this.high = frame?.highPresence ?? 0;
  }

  dispose(): void {
    this.stop();
    this.visible = false;
    this.events.abort();
    this.resize.disconnect();
    if (!this.state) return;
    const { gl, program, textures } = this.state;
    for (const texture of textures) gl.deleteTexture(texture);
    gl.deleteProgram(program);
    this.state = null;
  }

  private setup(): void {
    if (this.started) return;
    this.started = true;
    this.fallback.decode().then(() => this.initialize()).catch(() => { this.fallback.hidden = false; });
  }

  private initialize(): void {
    if (this.events.signal.aborted) return;
    const gl = this.canvas.getContext("webgl2", { alpha: true, premultipliedAlpha: false, antialias: false, powerPreference: "low-power" });
    if (!gl) return;
    const program = gl.createProgram();
    if (!program) return;
    const vs = compile(gl, gl.VERTEX_SHADER, vertex);
    const fs = compile(gl, gl.FRAGMENT_SHADER, fragment);
    gl.attachShader(program, vs);
    gl.attachShader(program, fs);
    gl.linkProgram(program);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      gl.deleteProgram(program);
      return;
    }
    gl.useProgram(program);
    const textures = [upload(gl, 0, this.fallback)];
    for (const [index, regions] of mattes.entries()) textures.push(upload(gl, index + 1, matte(regions)));
    gl.uniform1i(gl.getUniformLocation(program, "uArt"), 0);
    gl.uniform1iv(gl.getUniformLocation(program, "uMattes[0]"), [1, 2, 3, 4, 5]);
    gl.uniform1fv(gl.getUniformLocation(program, "uWeight[0]"), tokens.e2ParallaxLayers.map(layer => layer.parallaxFactor));
    this.state = {
      gl, program, textures,
      clock: gl.getUniformLocation(program, "uClock"),
      pointer: gl.getUniformLocation(program, "uPointer"),
      low: gl.getUniformLocation(program, "uLow"),
      high: gl.getUniformLocation(program, "uHigh"),
    };
    this.canvas.hidden = false;
    this.fallback.hidden = true;
    this.schedule();
  }

  private stop(): void {
    if (this.frame !== null) cancelAnimationFrame(this.frame);
    this.frame = null;
  }

  private schedule(): void {
    if (this.visible && this.state && this.frame === null) this.frame = requestAnimationFrame(this.draw);
  }

  private readonly draw = (now: number): void => {
    this.frame = null;
    if (!this.visible || !this.state) return;
    const { gl, program, clock, pointer, low, high } = this.state;
    const size = Math.min(640, Math.round(this.container.clientWidth * Math.min(2, devicePixelRatio)));
    if (size < 2) return;
    if (this.canvas.width !== size) this.canvas.width = this.canvas.height = size;
    gl.viewport(0, 0, size, size);
    gl.useProgram(program);
    gl.uniform1f(clock, this.reduced ? 0 : now / 1000);
    gl.uniform2f(pointer, this.reduced ? 0 : this.pointer.x, this.reduced ? 0 : this.pointer.y);
    gl.uniform1f(low, this.reduced ? 0 : this.low);
    gl.uniform1f(high, this.reduced ? 0 : this.high);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    if (!this.reduced) this.schedule();
  };
}
