/**
 * This is the base config for vite.
 * When building, the adapter config is used which loads this file and extends it.
 */
import { defineConfig, type UserConfig } from "vite";
import { qwikVite } from "@builder.io/qwik/optimizer";
import { qwikCity } from "@builder.io/qwik-city/vite";
import tsconfigPaths from "vite-tsconfig-paths";
import tailwindcss from "@tailwindcss/vite";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import pkg from "./package.json";

type PkgDep = Record<string, string>;
const { dependencies = {}, devDependencies = {} } = pkg as any as {
  dependencies: PkgDep;
  devDependencies: PkgDep;
  [key: string]: unknown;
};
errorOnDuplicatesPkgDeps(devDependencies, dependencies);

/**
 * Note that Vite normally starts from `index.html` but the qwikCity plugin makes start at `src/entry.ssr.tsx` instead.
 */
export default defineConfig(({ command, mode }): UserConfig => {
  return {
    plugins: [qwikCity(), qwikVite(), tsconfigPaths({ root: "." }), tailwindcss()],
    define: {
      __PAGES__: JSON.stringify(pageDates()),
    },
    // This tells Vite which dependencies to pre-build in dev mode.
    optimizeDeps: {
      // Put problematic deps that break bundling here, mostly those with binaries.
      // For example ['better-sqlite3'] if you use that in server functions.
      exclude: [],
    },

    /**
     * This is an advanced setting. It improves the bundling of your server code. To use it, make sure you understand when your consumed packages are dependencies or dev dependencies. (otherwise things will break in production)
     */
    // ssr:
    //   command === "build" && mode === "production"
    //     ? {
    //         // All dev dependencies should be bundled in the server build
    //         noExternal: Object.keys(devDependencies),
    //         // Anything marked as a dependency will not be bundled
    //         // These should only be production binary deps (including deps of deps), CLI deps, and their module graph
    //         // If a dep-of-dep needs to be external, add it here
    //         // For example, if something uses `bcrypt` but you don't have it as a dep, you can write
    //         // external: [...Object.keys(dependencies), 'bcrypt']
    //         external: Object.keys(dependencies),
    //       }
    //     : undefined,

    server: {
      headers: {
        // Don't cache the server response in dev mode
        "Cache-Control": "public, max-age=0",
      },
    },
    preview: {
      headers: {
        // Do cache the server response in preview (non-adapter production build)
        "Cache-Control": "public, max-age=600",
      },
    },
  };
});

// *** utils ***

/**
 * Every page, keyed by the path it is served at ("/docs/graphql/"), with the
 * date (YYYY-MM-DD) its content last changed in git, or null where git has
 * no record of it yet.
 *
 * A page's content is its route file and the `~/` modules it imports, followed
 * through their own imports: a comparison page is a few lines that hand
 * src/data/comparisons.ts to a component, and its date has to move when that
 * file does. Of those, a component shared by several pages is their layout
 * rather than their content -- restyling the code block is not news about
 * every page that shows one -- so it counts only for a page that has it to
 * itself. Data counts wherever it is read. Pages show the date as "Last
 * updated", put it in their structured data, and the sitemap reads it.
 *
 * A shallow clone knows only its last commit, and would date every page to
 * it; then, as with no git at all, every page is left undated rather than
 * claiming a freshness it does not have.
 */
function pageDates(): Record<string, string | null> {
  const fileDates = gitFileDates();
  const closures = new Map(
    routeFiles("src/routes").map((route) => [route, moduleClosure(route)]),
  );
  const pagesUsing: Record<string, number> = {};
  for (const closure of closures.values())
    for (const file of closure) pagesUsing[file] = (pagesUsing[file] ?? 0) + 1;
  const isContent = (file: string) =>
    file.startsWith("src/data/") || pagesUsing[file] === 1;

  const pages: Record<string, string | null> = {};
  for (const [route, closure] of closures) {
    let latest: string | null = null;
    for (const file of closure) {
      if (!isContent(file)) continue;
      const date = fileDates[file];
      if (date && (!latest || date > latest)) latest = date;
    }
    const path = `/${route.slice("src/routes/".length, -"index.tsx".length)}`;
    pages[path] = latest;
  }
  return pages;
}

/** The last commit date of every file under src, keyed "src/…". */
function gitFileDates(): Record<string, string> {
  const git = (args: string[]) =>
    execFileSync("git", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  let log: string;
  try {
    if (git(["rev-parse", "--is-shallow-repository"]).trim() === "true") {
      console.warn("page dates: shallow clone, leaving every page undated");
      return {};
    }
    log = git(["log", "--format=@%cs", "--name-only", "--", "src"]);
  } catch {
    return {};
  }
  const dates: Record<string, string> = {};
  let date = "";
  for (const line of log.split("\n")) {
    if (line.startsWith("@")) {
      date = line.slice(1);
      continue;
    }
    const file = line.replace(/^website\//, "");
    // Newest first, so the first date seen for a file is its latest.
    if (file.startsWith("src/") && !(file in dates)) dates[file] = date;
  }
  return dates;
}

/** Every route's index.tsx under `dir`. */
function routeFiles(dir: string): string[] {
  return readdirSync(dir, { recursive: true, encoding: "utf8" })
    .filter((f) => f === "index.tsx" || f.endsWith("/index.tsx"))
    .map((f) => `${dir}/${f}`);
}

/** `file` and every `~/` module it imports, directly or through others. */
function moduleClosure(file: string, seen = new Set<string>()): Set<string> {
  if (seen.has(file)) return seen;
  seen.add(file);
  const source = readFileSync(file, "utf8");
  for (const [, spec] of source.matchAll(/from\s+"~\/([^"]+)"/g)) {
    const target = ["", ".ts", ".tsx", "/index.ts", "/index.tsx"]
      .map((ext) => `src/${spec}${ext}`)
      .find((candidate) => existsSync(candidate) && statSync(candidate).isFile());
    if (target && /\.tsx?$/.test(target)) moduleClosure(target, seen);
  }
  return seen;
}

/**
 * Function to identify duplicate dependencies and throw an error
 * @param {Object} devDependencies - List of development dependencies
 * @param {Object} dependencies - List of production dependencies
 */
function errorOnDuplicatesPkgDeps(
  devDependencies: PkgDep,
  dependencies: PkgDep,
) {
  let msg = "";
  // Create an array 'duplicateDeps' by filtering devDependencies.
  // If a dependency also exists in dependencies, it is considered a duplicate.
  const duplicateDeps = Object.keys(devDependencies).filter(
    (dep) => dependencies[dep],
  );

  // include any known qwik packages
  const qwikPkg = Object.keys(dependencies).filter((value) =>
    /qwik/i.test(value),
  );

  // any errors for missing "qwik-city-plan"
  // [PLUGIN_ERROR]: Invalid module "@qwik-city-plan" is not a valid package
  msg = `Move qwik packages ${qwikPkg.join(", ")} to devDependencies`;

  if (qwikPkg.length > 0) {
    throw new Error(msg);
  }

  // Format the error message with the duplicates list.
  // The `join` function is used to represent the elements of the 'duplicateDeps' array as a comma-separated string.
  msg = `
    Warning: The dependency "${duplicateDeps.join(", ")}" is listed in both "devDependencies" and "dependencies".
    Please move the duplicated dependencies to "devDependencies" only and remove it from "dependencies"
  `;

  // Throw an error with the constructed message.
  if (duplicateDeps.length > 0) {
    throw new Error(msg);
  }
}
