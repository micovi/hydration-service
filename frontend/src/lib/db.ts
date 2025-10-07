// db.ts
import Dexie, { type EntityTable } from "dexie";

interface ProcessRecord {
  name: string;
  id: string;
  type: string;
  checkpoint: string | null;
  once: string | null;
  every: string | null;
}

const db = new Dexie("ProcessesDatabase") as Dexie & {
  processes: EntityTable<
    ProcessRecord,
    "id" // primary key "id" (for the typings only)
  >;
};

// Schema declaration:
db.version(1).stores({
  processes: "++id, name, type, checkpoint, once, every", // primary key "id" (for the runtime!)
});

export type { ProcessRecord };
export { db };
