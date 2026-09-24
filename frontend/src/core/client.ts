import * as Comlink from "comlink";
import type { CoreApi } from "./worker";

const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
export const core = Comlink.wrap<CoreApi>(worker);
