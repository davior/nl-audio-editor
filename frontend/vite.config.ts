import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  worker: { format: "es" },
  server: { port: 5173, strictPort: true, fs: { allow: [".."] } },
  build: { target: "es2022", sourcemap: true },
  test: { include: ["src/**/*.test.ts"] },
});
