/**
 * Just enough highlighting for the snippets a post shows: strings, keys,
 * comments, keywords, directives and numbers. No dependency, and nothing runs
 * in the browser -- the spans are rendered on the server like the rest of the
 * page.
 */
export type Lang = "sql" | "bash" | "json" | "graphql" | "yaml" | "text";

export interface Token {
  text: string;
  kind?:
    | "string"
    | "key"
    | "comment"
    | "keyword"
    | "directive"
    | "number"
    | "flag"
    | "punct";
}

const SQL_KEYWORDS =
  "CREATE|TABLE|PRIMARY|KEY|REFERENCES|NOT|NULL|DEFAULT|ALTER|ENABLE|ROW|LEVEL|SECURITY|POLICY|ON|FOR|SELECT|TO|USING|INSERT|INTO|VALUES|GRANT|ROLE|LOGIN|NOINHERIT|PASSWORD|int|text|boolean|true|false";
const GRAPHQL_KEYWORDS =
  "query|mutation|subscription|type|fragment|on|extend|schema|input|enum";

const RULES: Record<Exclude<Lang, "text">, [Token["kind"], string][]> = {
  sql: [
    ["comment", "--[^\\n]*"],
    ["string", "'(?:[^']|'')*'"],
    ["keyword", `\\b(?:${SQL_KEYWORDS})\\b`],
    ["number", "\\b\\d+\\b"],
  ],
  bash: [
    ["comment", "(?<=^|\\s)#[^\\n]*"],
    ["string", "\"(?:[^\"\\\\]|\\\\.)*\"|'[^']*'"],
    ["flag", "(?<=\\s)--?[A-Za-z][\\w-]*"],
    ["key", "\\b[A-Z][A-Z0-9_]+(?==)"],
    [
      "keyword",
      "^(?:docker|rover|router|git|cd|curl)\\b|(?<=\\n)(?:docker|rover|router|git|cd|curl)\\b",
    ],
  ],
  json: [
    ["key", '"(?:[^"\\\\]|\\\\.)*"(?=\\s*:)'],
    ["string", '"(?:[^"\\\\]|\\\\.)*"'],
    ["keyword", "\\b(?:true|false|null)\\b"],
    ["number", "-?\\b\\d+(?:\\.\\d+)?\\b"],
    ["punct", "[{}\\[\\],:]"],
  ],
  graphql: [
    ["comment", "#[^\\n]*"],
    ["string", '"(?:[^"\\\\]|\\\\.)*"'],
    ["directive", "@\\w+"],
    ["keyword", `\\b(?:${GRAPHQL_KEYWORDS})\\b`],
    ["key", "\\b__typename\\b|\\b_entities\\b|\\b_service\\b"],
    ["number", "\\b\\d+\\b"],
    ["punct", "[{}()\\[\\]!:,]"],
  ],
  yaml: [
    ["comment", "#[^\\n]*"],
    ["key", "[A-Za-z_][\\w.-]*(?=:(?:\\s|$))"],
    ["string", "\"(?:[^\"\\\\]|\\\\.)*\"|'[^']*'"],
    ["number", "=?\\d+(?:\\.\\d+)*"],
    ["punct", "^\\s*-(?=\\s)|(?<=\\n)\\s*-(?=\\s)"],
  ],
};

const compiled = new Map<Lang, RegExp>();

function pattern(lang: Exclude<Lang, "text">): RegExp {
  let re = compiled.get(lang);
  if (!re) {
    re = new RegExp(
      RULES[lang].map(([, source]) => `(${source})`).join("|"),
      "gm",
    );
    compiled.set(lang, re);
  }
  return re;
}

export function highlight(code: string, lang: Lang): Token[] {
  if (lang === "text") return [{ text: code }];
  const rules = RULES[lang];
  const re = pattern(lang);
  re.lastIndex = 0;
  const tokens: Token[] = [];
  let last = 0;
  for (let m = re.exec(code); m; m = re.exec(code)) {
    if (m[0] === "") {
      re.lastIndex++;
      continue;
    }
    if (m.index > last) tokens.push({ text: code.slice(last, m.index) });
    const group = m.slice(1).findIndex((g) => g !== undefined);
    tokens.push({ text: m[0], kind: rules[group]?.[0] });
    last = m.index + m[0].length;
  }
  if (last < code.length) tokens.push({ text: code.slice(last) });
  return tokens;
}

export const TOKEN_CLASS: Record<NonNullable<Token["kind"]>, string> = {
  string: "text-emerald-300",
  key: "text-sky-300",
  comment: "text-neutral-500 italic",
  keyword: "text-primary-300",
  directive: "text-violet-300",
  number: "text-amber-300",
  flag: "text-violet-300",
  punct: "text-neutral-500",
};
