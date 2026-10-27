import { component$, useSignal } from "@builder.io/qwik";
import { highlight, TOKEN_CLASS, type Lang } from "./highlight";

const LANG_NAME: Record<Lang, string> = {
  sql: "SQL",
  bash: "Shell",
  json: "JSON",
  graphql: "GraphQL",
  yaml: "YAML",
  text: "",
};

export const Code = component$<{ label: string; code: string; lang?: Lang }>(
  ({ label, code, lang = "text" }) => {
    const copied = useSignal(false);
    return (
      <figure class="group mb-8 overflow-hidden rounded-xl bg-neutral-950 shadow-lg ring-1 shadow-neutral-900/10 ring-neutral-900/10">
        <figcaption class="flex items-center gap-3 border-b border-white/10 bg-white/[0.03] px-4 py-2.5">
          <span class="flex gap-1.5" aria-hidden="true">
            <span class="h-2.5 w-2.5 rounded-full bg-white/15" />
            <span class="h-2.5 w-2.5 rounded-full bg-white/15" />
            <span class="h-2.5 w-2.5 rounded-full bg-white/15" />
          </span>
          <span class="min-w-0 flex-1 truncate font-mono text-xs text-neutral-400">
            {label}
          </span>
          {LANG_NAME[lang] && (
            <span class="hidden text-[0.6875rem] font-medium tracking-wide text-neutral-500 uppercase sm:inline">
              {LANG_NAME[lang]}
            </span>
          )}
          <button
            type="button"
            class="rounded-md px-2 py-1 text-xs font-medium text-neutral-400 transition-colors hover:bg-white/10 hover:text-white"
            aria-label={`Copy ${label}`}
            onClick$={async () => {
              await navigator.clipboard.writeText(code);
              copied.value = true;
              setTimeout(() => (copied.value = false), 1500);
            }}
          >
            {copied.value ? "Copied" : "Copy"}
          </button>
        </figcaption>
        <pre class="overflow-x-auto px-5 py-4 text-[0.8125rem] leading-6">
          <code class="text-neutral-200">
            {highlight(code, lang).map((t, i) =>
              t.kind ? (
                <span key={i} class={TOKEN_CLASS[t.kind]}>
                  {t.text}
                </span>
              ) : (
                t.text
              ),
            )}
          </code>
        </pre>
      </figure>
    );
  },
);
