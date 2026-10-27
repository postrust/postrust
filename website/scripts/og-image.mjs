// Renders public/og.png, the 1200x630 card social sites and chat apps show
// for a link to postrust.org. Run with `node scripts/og-image.mjs` after
// changing the wording.
import sharp from "sharp";

const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630">
  <defs>
    <linearGradient id="mark" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#f5912c"/><stop offset="100%" stop-color="#d56a10"/>
    </linearGradient>
  </defs>
  <rect width="1200" height="630" fill="#171717"/>
  <g transform="translate(80 80) scale(3)">
    <rect width="32" height="32" rx="6" fill="url(#mark)"/>
    <path d="M17.5 6L9 18h7l-1 8 9-12h-7l1-8z" fill="white" stroke="white" stroke-width="1" stroke-linejoin="round"/>
  </g>
  <text x="200" y="152" font-family="DejaVu Sans, Arial, sans-serif" font-size="56" font-weight="700" fill="#ffffff">Postrust</text>
  <text x="80" y="330" font-family="DejaVu Sans, Arial, sans-serif" font-size="64" font-weight="700" fill="#ffffff">PostgreSQL as REST and</text>
  <text x="80" y="410" font-family="DejaVu Sans, Arial, sans-serif" font-size="64" font-weight="700" fill="#f5912c">GraphQL APIs, in Rust</text>
  <text x="80" y="500" font-family="DejaVu Sans, Arial, sans-serif" font-size="30" fill="#a3a3a3">One binary · Docker · Kubernetes · AWS Lambda · Cloudflare Workers</text>
  <text x="80" y="560" font-family="DejaVu Sans, Arial, sans-serif" font-size="26" fill="#737373">postrust.org · open source</text>
</svg>`;

await sharp(Buffer.from(svg))
  .png()
  .toFile(new URL("../public/og.png", import.meta.url).pathname);
