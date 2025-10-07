import { Hono } from "hono";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import Redis from "ioredis";
import { Cron } from "croner";
import { cors } from "hono/cors";

const app = new Hono();

app.use("/*", cors());

// No connection is made until a command is executed
const redis = new Redis(process.env.REDIS_URL || "redis://localhost:6379");

const HB_URL = process.env.HB_URL || "http://65.108.7.125:8734";

const runners = {
  latestSlot: false,
};

async function fetchLatestSlot(processId: string): Promise<number> {
  // Fetch the latest slot from HB
  const response = await fetch(
    `${HB_URL}/${processId}~process@1.0/slot/current`
  );

  if (!response.ok) {
    throw new Error(`HTTP error! status: ${response.status}`);
  }
  const data = await response.text();

  const latestSlot = Number.parseInt(data, 10);

  // if latestslot is different from the one in redis, update it
  const currentSlot = await redis.hget(`process:${processId}`, "latest_slot");

  if (currentSlot && Number.parseInt(currentSlot, 10) === latestSlot) {
    return latestSlot;
  }

  // Store the latest slot in Redis
  await redis.hset(`process:${processId}`, "latest_slot", latestSlot);
  console.log(
    `Stored latest slot ${latestSlot} for process ${processId} in Redis`
  );

  // Optionally, you can also store a timestamp of when this was last updated
  const timestamp = new Date().toISOString();
  await redis.hset(`process:${processId}`, "latest_slot_timestamp", timestamp);
  console.log(
    `Updated latest_slot_timestamp for process ${processId} to ${timestamp}`
  );

  return latestSlot;
}

async function fetchComputeAtSlot(processId: string): Promise<number> {
  // Fetch the compute/at-slot from HB
  const response = await fetch(
    `${HB_URL}/${processId}~process@1.0/compute/at-slot`
  );

  if (!response.ok) {
    throw new Error(`HTTP error! status: ${response.status}`);
  }
  const data = await response.text();

  const computeAtSlot = Number.parseInt(data, 10);

  // if computeAtSlot is different from the one in redis, update it
  const currentComputeAtSlot = await redis.hget(
    `process:${processId}`,
    "compute_at_slot"
  );

  if (
    currentComputeAtSlot &&
    Number.parseInt(currentComputeAtSlot, 10) === computeAtSlot
  ) {
    return computeAtSlot;
  }

  // Store the compute/at-slot in Redis
  await redis.hset(`process:${processId}`, "compute_at_slot", computeAtSlot);
  console.log(
    `Stored compute/at-slot ${computeAtSlot} for process ${processId} in Redis`
  );

  // Store timestamp of when this was last updated
  const timestamp = new Date().toISOString();
  await redis.hset(
    `process:${processId}`,
    "compute_at_slot_timestamp",
    timestamp
  );
  console.log(
    `Updated compute_at_slot_timestamp for process ${processId} to ${timestamp}`
  );
  return computeAtSlot;
}

// Cron job to query slots every minute
new Cron("*/1 * * * *", async () => {
  console.log("Running cron job to fetch latest slots from HB");

  if (runners.latestSlot) {
    console.log("Latest slot cron job is already running, skipping this run");
    return;
  }

  runners.latestSlot = true;

  try {
    const processes = await redis.smembers("processes");

    for (const processId of processes) {
      await fetchLatestSlot(processId);
      await fetchComputeAtSlot(processId);
    }
  } catch (error) {
    console.error("Error fetching latest slot from HB:", error);
  } finally {
    runners.latestSlot = false;
  }
});

app.get("/load-processes", async (c) => {
  const rawData = await readFile(
    resolve(process.cwd(), "src/processes.json"),
    "utf8"
  );
  const processes = JSON.parse(rawData) as {
    name: string;
    id: string;
    type: string;
  }[];

  for (const process of processes) {
    // Check if already exists
    const exists = await redis.sismember("processes", process.id);
    if (exists) {
      console.log(`Process ${process.id} already exists, skipping`);
      continue;
    }

    await redis.sadd("processes", process.id);
    await redis.hmset(`process:${process.id}`, [
      "name",
      process.name,
      "type",
      process.type,
      "id",
      process.id,
      "latest_slot",
      "0",
      "latest_slot_timestamp",
      new Date(0).toISOString(),
      "compute_at_slot",
      "0",
      "compute_at_slot_timestamp",
      new Date(0).toISOString(),
    ]);
  }

  return c.json({ message: "Processes loaded", count: processes.length });
});

app.get("/compute/at-slot/:processId", async (c) => {
  const { processId } = c.req.param();

  // Trigger also an immediate update
  try {
    await fetchComputeAtSlot(processId);
  } catch (error) {
    console.error(
      `Error fetching compute/at-slot for process ${processId} from HB:`,
      error
    );
  }

  const computeAtSlot = await redis.hget(
    `process:${processId}`,
    "compute_at_slot"
  );
  const computeAtSlotTimestamp = await redis.hget(
    `process:${processId}`,
    "compute_at_slot_timestamp"
  );

  return c.json({
    processId,
    computeAtSlot: computeAtSlot ? Number.parseInt(computeAtSlot, 10) : null,
    computeAtSlotTimestamp: computeAtSlotTimestamp || null,
  });
});

app.get("/slot/current/:processId", async (c) => {
  const { processId } = c.req.param();

  // Trigger also an immediate update
  try {
    await fetchLatestSlot(processId);
  } catch (error) {
    console.error(
      `Error fetching latest slot for process ${processId} from HB:`,
      error
    );
  }

  const latestSlot = await redis.hget(`process:${processId}`, "latest_slot");
  const latestSlotTimestamp = await redis.hget(
    `process:${processId}`,
    "latest_slot_timestamp"
  );

  return c.json({
    processId,
    latestSlot: latestSlot ? Number.parseInt(latestSlot, 10) : null,
    latestSlotTimestamp: latestSlotTimestamp || null,
  });
});

app.get("/cron/once/:processId", async (c) => {
  const { processId } = c.req.param();

  // Trigger once cron job via HB
  const rs = await fetch(
    `${HB_URL}/~cron@1.0/once?cron-path=/${processId}~process@1.0/now`
  );

  if (!rs.ok) {
    const rtext = await rs.text();
    console.log("Response text:", rtext);
    throw new Error(`HTTP error! status: ${rs.status}`);
  }

  const responseId = await rs.text();

  // Store the "once" ID in Redis
  await redis.hset(`process:${processId}`, "taskId", responseId);
  console.log(`Stored once ID ${responseId} for process ${processId} in Redis`);

  return c.text(responseId);
});

app.get("/cron/every/:processId", async (c) => {
  const { processId } = c.req.param();

  // Trigger every cron job via HB
  const rs = await fetch(
    `${HB_URL}/~cron@1.0/every?cron-path=/${processId}~process@1.0/now&interval=5-minutes`
  );

  if (!rs.ok) {
    const rtext = await rs.text();
    console.log("Response text:", rtext);
    throw new Error(`HTTP error! status: ${rs.status}`);
  }

  const responseId = await rs.text();

  // Store the "every" ID in Redis
  await redis.hset(`process:${processId}`, "taskId", responseId);
  console.log(
    `Stored every ID ${responseId} for process ${processId} in Redis`
  );

  return c.text(responseId);
});

app.get("/cron/stop/:processId", async (c) => {
  const { processId } = c.req.param();

  const processData = await redis.hgetall(`process:${processId}`);
  const taskId = processData.taskId;

  if (!taskId) {
    return c.text("No 'once' or 'every' task to stop.", 400);
  }

  // Stop cron job via HB
  const rs = await fetch(`${HB_URL}/~cron@1.0/stop?task=${taskId}`);

  if (!rs.ok) {
    const rtext = await rs.text();
    console.log("Response text:", rtext);

    if (rtext.includes("Task not found")) {
      // If the task does not exist, we can safely remove it from Redis
      await redis.hdel(`process:${processId}`, "taskId");
      console.log(`Cleared taskId for process ${processId} in Redis`);
      return c.text("Cron job not found. Cleared taskId from Redis.");
    }

    throw new Error(`HTTP error! status: ${rs.status}`);
  }

  // Remove the "once" or "every" ID from Redis
  await redis.hdel(`process:${processId}`, "taskId");
  console.log(`Cleared taskId for process ${processId} in Redis`);

  return c.text("Cron job stopped.");
});

app.get("/", async (c) => {
  const processes = await redis.smembers("processes");
  const processDetails = await Promise.all(
    processes.map((processId) => redis.hgetall(`process:${processId}`))
  );

  // Sort processes by type ascending
  processDetails.sort((a, b) => b.type.localeCompare(a.type));

  return c.json({
    message: "Hydration Service Backend",
    processes: processDetails,
  });
});

app.post("/add-process", async (c) => {
  const { name, id, type } = await c.req.json();

  if (!name || !id || !type) {
    return c.json({ error: "Missing required fields" }, 400);
  }

  // Check if already exists
  const exists = await redis.sismember("processes", id);
  if (exists) {
    return c.json({ error: "Process already exists" }, 400);
  }

  await redis.sadd("processes", id);
  await redis.hmset(`process:${id}`, [
    "name",
    name,
    "type",
    type,
    "id",
    id,
    "latest_slot",
    "0",
    "latest_slot_timestamp",
    new Date(0).toISOString(),
    "compute_at_slot",
    "0",
    "compute_at_slot_timestamp",
    new Date(0).toISOString(),
  ]);

  return c.json({ message: "Process added", process: { name, id, type } });
});

export default {
  port: 8081,
  fetch: app.fetch,
};
