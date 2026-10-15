export const DEV_CONFIG = "src-tauri/tauri.dev.conf.json";

/** Args do `tauri` com a config de dev injetada em `tauri dev`, a menos que
 * quem chama já tenha passado uma `--config`. */
export function withDevConfig(args: string[]): string[] {
  if (args[0] !== "dev") return args;
  if (args.some((a) => a === "--config" || a === "-c" || a.startsWith("--config="))) {
    return args;
  }
  return ["dev", "--config", DEV_CONFIG, ...args.slice(1)];
}
