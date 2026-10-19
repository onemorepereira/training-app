import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [sveltekit()],
  // Prevent Vite from obscuring Rust errors
  clearScreen: false,
  server: {
    // Tauri expects a fixed port
    strictPort: true
  },
  test: {
    // Pin a non-UTC zone so local-calendar logic is tested the same on every machine.
    // Node re-reads process.env.TZ at runtime, so this applies inside test workers.
    env: { TZ: 'America/Chicago' },
  },
});
