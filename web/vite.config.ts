import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  server: {
    // The library is read from ../compose at dev time.
    fs: { allow: [".."] },
  },
});
