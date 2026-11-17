export type SkyframeQualityPreset = "performance" | "balanced" | "high" | "ultra";

export interface SkyframeQualitySettings {
  resolutionScale: number;
  pixelRatioLimit: number;
  maxFps: number;
  atlasAnisotropy: number;
  glassRefraction: boolean;
  glassTransmission: number;
  edgeGlow: number;
  floorGrid: boolean;
  idleMotion: boolean;
}

export const qualityPresets: Readonly<Record<SkyframeQualityPreset, SkyframeQualitySettings>> = {
  performance: {
    resolutionScale: 0.82, pixelRatioLimit: 1, maxFps: 30, atlasAnisotropy: 1,
    glassRefraction: false, glassTransmission: 0.5, edgeGlow: 0.35, floorGrid: true, idleMotion: false,
  },
  balanced: {
    resolutionScale: 1, pixelRatioLimit: 1.5, maxFps: 60, atlasAnisotropy: 2,
    glassRefraction: true, glassTransmission: 0.65, edgeGlow: 0.85, floorGrid: true, idleMotion: true,
  },
  high: {
    resolutionScale: 1, pixelRatioLimit: 1.85, maxFps: 60, atlasAnisotropy: 4,
    glassRefraction: true, glassTransmission: 0.75, edgeGlow: 1, floorGrid: true, idleMotion: true,
  },
  ultra: {
    resolutionScale: 1, pixelRatioLimit: 2, maxFps: 120, atlasAnisotropy: 8,
    glassRefraction: true, glassTransmission: 0.85, edgeGlow: 1.15, floorGrid: true, idleMotion: true,
  },
};

export const superPerformance: SkyframeQualitySettings = {
  ...qualityPresets.performance,
  resolutionScale: 0.65,
  maxFps: 24,
  floorGrid: false,
  edgeGlow: 0,
};

function within(value: number | undefined, min: number, max: number, fallback: number) {
  return value !== undefined && Number.isFinite(value) ? Math.max(min, Math.min(max, value)) : fallback;
}

export function qualitySettings(preset: SkyframeQualityPreset, changes: Partial<SkyframeQualitySettings>) {
  const base = qualityPresets[preset];
  return {
    ...base, ...changes,
    resolutionScale: within(changes.resolutionScale, 0.5, 1, base.resolutionScale),
    pixelRatioLimit: within(changes.pixelRatioLimit, 0.7, 2, base.pixelRatioLimit),
    maxFps: within(changes.maxFps, 20, 120, base.maxFps),
    atlasAnisotropy: within(changes.atlasAnisotropy, 1, 8, base.atlasAnisotropy),
    glassTransmission: within(changes.glassTransmission, 0.4, 1, base.glassTransmission),
    edgeGlow: within(changes.edgeGlow, 0, 1.25, base.edgeGlow),
  } satisfies SkyframeQualitySettings;
}
