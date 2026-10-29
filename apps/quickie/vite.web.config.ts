import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The phone web client. Built to dist-web/, bundled into the desktop app as a
// resource and served by the desktop to paired phones.
export default defineConfig({
  root: "web",
  plugins: [svelte({ configFile: "../svelte.config.js" })],
  build: { outDir: "../dist-web", emptyOutDir: true },
});
