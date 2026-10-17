import { cloudflarePagesAdapter } from "@builder.io/qwik-city/adapters/cloudflare-pages/vite";
import { extendConfig } from "@builder.io/qwik-city/vite";
import baseConfig from "../../vite.config";

export default extendConfig(baseConfig, () => {
  return {
    build: {
      ssr: true,
      rollupOptions: {
        input: ["src/entry.cloudflare-pages.tsx", "@qwik-city-plan"],
      },
    },
    plugins: [
      // The adapter writes a sitemap of the pages it pre-renders, which is
      // none, and that empty file would overwrite public/sitemap.xml.
      cloudflarePagesAdapter({ ssg: { sitemapOutFile: null } }),
    ],
  };
});
