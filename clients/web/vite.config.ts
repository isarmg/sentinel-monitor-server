import { createSarmgReactViteConfig } from "@sarmg/web-toolchain/vite";
import { mergeConfig } from "vite";
import { fileURLToPath } from "node:url";

export default mergeConfig(createSarmgReactViteConfig(), {
  resolve: { alias: [{ find: /^@sarmg\/admin-ui$/, replacement: fileURLToPath(new URL("./shell/ui.js", import.meta.url)) }] },
  server: {
    port: 5173,
    proxy: {
      "/api/v2": "http://127.0.0.1:8080",
      "/healthz": "http://127.0.0.1:8080",
      "/readyz": "http://127.0.0.1:8080",
      "/media-webrtc": {
        target: "http://127.0.0.1:8889",
        rewrite: (path: string) => path.replace(/^\/media-webrtc/, ""),
      },
      "/media-hls": {
        target: "http://127.0.0.1:8888",
        rewrite: (path: string) => path.replace(/^\/media-hls/, ""),
      },
    },
  },
});
