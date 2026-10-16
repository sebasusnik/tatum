// The library the app ships: the repo's `compose/` directory, bundled as
// text, and over it whatever the browser saved -- voices written from the
// screen and the session as it was left -- kept in localStorage by path.

const bundled = import.meta.glob("../../compose/**/*.synth", { query: "?raw", import: "default", eager: true }) as Record<
  string,
  string
>;

const PREFIX = "../../compose/";
const STORE = "tatum.compose.files";

/** Every file of the repo's library, by the path a session names it. */
export function shipped(): Record<string, string> {
  const files: Record<string, string> = {};
  for (const [key, text] of Object.entries(bundled)) {
    if (key.startsWith(PREFIX)) files[key.slice(PREFIX.length)] = text;
  }
  return files;
}

/** What this browser saved, by path. */
export function stored(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(STORE) ?? "{}") as Record<string, string>;
  } catch {
    return {};
  }
}

export function store(path: string, text: string): void {
  const files = stored();
  files[path] = text;
  localStorage.setItem(STORE, JSON.stringify(files));
}

export function forgetStored(): void {
  localStorage.removeItem(STORE);
}

/** A file offered to the disk, as the terminal would have written it. */
export function download(path: string, text: string): void {
  const blob = new Blob([text], { type: "text/plain" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = path.split("/").pop() ?? "voice.synth";
  a.click();
  URL.revokeObjectURL(url);
}
