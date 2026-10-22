import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

export default defineConfig(({ mode }) => {
  const environment = loadEnv(mode, process.cwd(), "");
  const backendTarget = environment.VITE_PLATFORM_BFF_URL ?? "http://localhost:8080";

  return {
    plugins: [react()],
    server: {
      proxy: {
        "/v1": {
          target: backendTarget,
          changeOrigin: true,
          configure(proxy) {
            proxy.on("proxyReq", (proxyRequest) => {
              const target = new URL(backendTarget);
              proxyRequest.setHeader("origin", target.origin);
            });
          },
        },
      },
    },
  };
});
