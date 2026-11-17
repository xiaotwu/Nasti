import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";

export default defineConfig({
  base: "./",
  publicDir: fileURLToPath(new URL("../../assets/", import.meta.url)),
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  build: { target: "safari16" },
});
