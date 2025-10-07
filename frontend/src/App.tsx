import "./App.css";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Client, cacheExchange, fetchExchange, gql } from "urql";
import { useQueryState, parseAsBoolean } from "nuqs";
import { CheckSquare, RefreshCwIcon } from "lucide-react";
import { useLiveQuery } from "dexie-react-hooks";

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { cn } from "./lib/utils";
import { Button } from "./components/ui/button";
import { db, type ProcessRecord } from "./lib/db";

interface Process {
  name: string;
  id: string;
  type: string;
}

export const HB_URL = "/hydration-service/hb-node";
//export const HB_URL = "http://65.108.7.125:8734";

const FLP_QUERY = gql`
  query Transactions($id: String!) {
    transactions(
      first: 1
      tags: [
        { name: "Type", values: ["Checkpoint"] }
        { name: "Process", values: [$id] }
      ]
    ) {
      edges {
        node {
          id
          tags {
            name
            value
          }
        }
      }
    }
  }
`;

const graphqlClient = new Client({
  url: "https://arweave-search.goldsky.com/graphql",
  exchanges: [cacheExchange, fetchExchange],
  preferGetMethod: false,
  fetchOptions: { method: "POST" },
});

function App() {
  const {
    data: processes,
    isLoading,
    error,
  } = useQuery<ProcessRecord[]>({
    queryKey: ["processes"],
    queryFn: async () => {
      const response = await fetch("/hydration-service/processes.json");

      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }

      const data = (await response.json()) as ProcessRecord[];

      for (const process of data) {
        const existing = await db.processes.get(process.id);
        if (!existing) {
          await db.processes.upsert(process.id, {
            ...process,
            checkpoint: null,
            once: null,
            every: null,
          });

          console.log(`Inserted process ${process.id} into IndexedDB`);
        }
      }

      return data;
    },
  });

  return (
    <div>
      <h1 className="text-4xl text-center my-4 font-mono">
        Node Hydration Service
      </h1>
      {isLoading && <p className="text-center">Loading...</p>}

      {error && (
        <p className="text-center text-red-500">Error loading processes</p>
      )}

      {processes && (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>ID</TableHead>
              <TableHead>Type</TableHead>
              <TableHead>slot/current</TableHead>
              <TableHead>compute/at-slot</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {processes.map((process) => (
              <ProcessRecordRow key={process.id} process={process} />
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  );
}

function ProcessRecordRow({ process }: { process: Process }) {
  const queryClient = useQueryClient();
  const [debug] = useQueryState("debug", parseAsBoolean.withDefault(false));

  const {
    data: checkpoint,
    isLoading: isLoadingCheckpoint,
    error: errorCheckpoint,
  } = useQuery<string | null>({
    queryKey: ["checkpoint", process.id],
    queryFn: async () => {
      const result = await graphqlClient
        .query(FLP_QUERY, { id: process.id })
        .toPromise();

      if (result.error) {
        throw new Error(result.error.message);
      }

      const edges = result.data?.transactions.edges;
      if (edges && edges.length > 0) {
        const processInDb = await db.processes.get(process.id);

        if (processInDb?.checkpoint === edges[0].node.id) {
          return edges[0].node.id;
        }

        await db.processes.upsert(process.id, { checkpoint: edges[0].node.id });

        return edges[0].node.id;
      }

      return null;
    },
    staleTime: 1000 * 60 * 5, // 5 minutes
  });

  const {
    data: computeAtSlot,
    isLoading: isLoadingComputeAtSlot,
    isFetching: isReloadingComputeAtSlot,
    error: errorComputeAtSlot,
  } = useQuery<string>({
    queryKey: ["computeAtSlot", process.id],
    queryFn: () =>
      fetch(`${HB_URL}/${process.id}~process@1.0/compute/at-slot`).then(
        (res) => {
          if (!res.ok) {
            throw new Error(`HTTP error! status: ${res.status}`);
          }
          return res.text();
        }
      ),
    staleTime: 1000 * 60, // 1 minute
  });

  const {
    data: latestSlot,
    isLoading: isLoadingLatestSlot,
    isFetching: isReloadingLatestSlot,
    error: errorLatestSlot,
  } = useQuery<string>({
    queryKey: ["latestSlot", process.id],
    queryFn: () =>
      fetch(`${HB_URL}/${process.id}~process@1.0/slot/current`).then((res) => {
        if (!res.ok) {
          throw new Error(`HTTP error! status: ${res.status}`);
        }
        return res.text();
      }),
    staleTime: 1000 * 60, // 1 minute
  });

  const loadCheckpointMutation = useMutation({
    mutationFn: async () => {
      const result = await fetch(
        `${HB_URL}/~genesis-wasm@1.0/import=${checkpoint}&process-id=${process.id}`
      );
      console.log(result.status, await result.text());
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["computeAtSlot", process.id],
      });
      queryClient.invalidateQueries({ queryKey: ["latestSlot", process.id] });
    },
  });

  const reloadData = () => {
    queryClient.invalidateQueries({ queryKey: ["computeAtSlot", process.id] });
    queryClient.invalidateQueries({ queryKey: ["latestSlot", process.id] });
  };

  return (
    <TableRow key={process.id}>
      <TableCell className="font-mono">{process.name}</TableCell>
      <TableCell className="font-mono">
        <a
          href={`https://ao.link/#/entity/${process.id}`}
          target="_blank"
          rel="noreferrer"
        >
          {process.id}
        </a>
      </TableCell>
      <TableCell>{process.type}</TableCell>
      <TableCell className="font-mono">
        {latestSlot ||
          (isLoadingLatestSlot ? "Loading..." : errorLatestSlot?.message)}
      </TableCell>
      <TableCell
        className={cn(
          "font-mono",
          latestSlot &&
            computeAtSlot &&
            (Number(computeAtSlot) < Number(latestSlot)
              ? "text-red-500"
              : "text-green-500")
        )}
      >
        {computeAtSlot ||
          (isLoadingComputeAtSlot ? "Loading..." : errorComputeAtSlot?.message)}
      </TableCell>
      <TableCell>
        <Button
          onClick={reloadData}
          type="button"
          disabled={isReloadingComputeAtSlot || isReloadingLatestSlot}
        >
          <RefreshCwIcon
            className={cn(
              isReloadingComputeAtSlot ||
                (isReloadingLatestSlot && "animate-spin")
            )}
          />
        </Button>
      </TableCell>
      {debug && (
        <TableCell className="flex flex-row gap-1">
          <OnceButton process={process} />
          <EveryButton process={process} />
          {checkpoint ? (
            <Button
              className="font-mono"
              onClick={() => loadCheckpointMutation.mutate()}
              type="button"
            >
              <CheckSquare />
            </Button>
          ) : isLoadingCheckpoint ? (
            "..."
          ) : (
            errorCheckpoint?.message
          )}
        </TableCell>
      )}
    </TableRow>
  );
}

function OnceButton({ process }: { process: Process }) {
  const hasStarted = useLiveQuery(async () => {
    const processInDb = await db.processes.get(process.id);
    return processInDb?.once !== null;
  }, [process.id]);

  const startMutation = useMutation({
    mutationFn: async () => {
      const rs = await fetch(
        `${HB_URL}/~cron@1.0/once?cron-path=/${process.id}~process@1.0/now`
      );

      const responseId = await rs.text();

      console.log(rs.status, responseId);

      return responseId;
    },
    onSuccess: async (responseId) => {
      const processInDb = await db.processes.get(process.id);

      if (processInDb?.once !== responseId) {
        await db.processes.upsert(process.id, { once: responseId });
      }

      console.log(
        "Triggered once for process:",
        process.id,
        "Response ID:",
        responseId
      );
    },
  });

  const stopMutation = useMutation({
    mutationFn: async () => {
      const processInDb = await db.processes.get(process.id);

      const taskId = processInDb?.once;

      if (!taskId) {
        throw new Error("No 'once' process to stop.");
      }

      const rs = await fetch(`${HB_URL}/~cron@1.0/stop?task=${taskId}`);

      if (!rs.ok) {
        const rtext = await rs.text();
        console.log("Response text:", rtext);

        if (rtext.includes("Task not found")) {
          await db.processes.upsert(process.id, { once: null });
          console.log("Cleared stale 'once' process for:", process.id);
          return;
        }

        throw new Error(`HTTP error! status: ${rs.status}`);
      }

      console.log(rs.status, await rs.text());
    },
    onSuccess: async () => {
      await db.processes.upsert(process.id, { once: null });
      console.log("Stopped 'once' process for:", process.id);
    },
  });

  if (hasStarted) {
    return (
      <Button
        className="font-mono"
        onClick={() => stopMutation.mutate()}
        disabled={
          stopMutation.isPending ||
          stopMutation.isSuccess ||
          stopMutation.isError
        }
        type="button"
      >
        {stopMutation.isPending ? "..." : "■"}
      </Button>
    );
  }

  return (
    <Button
      className="font-mono"
      onClick={() => startMutation.mutate()}
      disabled={
        startMutation.isPending ||
        startMutation.isSuccess ||
        startMutation.isError
      }
      type="button"
    >
      {startMutation.isSuccess ? (
        "1"
      ) : (
        <span>{startMutation.isPending ? "..." : "1"}</span>
      )}
    </Button>
  );
}

function EveryButton({ process }: { process: Process }) {
  const hasStarted = useLiveQuery(async () => {
    const processInDb = await db.processes.get(process.id);
    return processInDb?.every !== null;
  }, [process.id]);

  const startMutation = useMutation({
    mutationFn: async () => {
      const rs = await fetch(
        `${HB_URL}/~cron@1.0/every?cron-path=/${process.id}~process@1.0/now&interval=5-minutes`
      );

      const responseId = await rs.text();

      console.log(rs.status, responseId);

      return responseId;
    },
    onSuccess: async (responseId) => {
      const processInDb = await db.processes.get(process.id);

      if (processInDb?.every !== responseId) {
        await db.processes.upsert(process.id, { every: responseId });
      }

      console.log(
        "Set up every 5 minutes for process:",
        process.id,
        "Response ID:",
        responseId
      );
    },
  });

  const stopMutation = useMutation({
    mutationFn: async () => {
      const processInDb = await db.processes.get(process.id);

      const taskId = processInDb?.every;

      if (!taskId) {
        throw new Error("No 'every' process to stop.");
      }

      const rs = await fetch(`${HB_URL}/~cron@1.0/stop?task=${taskId}`);

      if (!rs.ok) {
        const rtext = await rs.text();
        console.log("Response text:", rtext);

        if (rtext.includes("Task not found")) {
          await db.processes.upsert(process.id, { every: null });
          console.log("Cleared stale 'every' process for:", process.id);
          return;
        }

        throw new Error(`HTTP error! status: ${rs.status}`);
      }

      console.log(rs.status, await rs.text());
    },
    onSuccess: async () => {
      await db.processes.upsert(process.id, { every: null });
      console.log("Stopped 'every' process for:", process.id);
    },
  });

  if (hasStarted) {
    return (
      <Button
        className="font-mono"
        onClick={() => stopMutation.mutate()}
        disabled={
          stopMutation.isPending ||
          stopMutation.isSuccess ||
          stopMutation.isError
        }
        type="button"
      >
        {stopMutation.isPending ? "..." : "■"}
      </Button>
    );
  }

  return (
    <Button
      className="font-mono"
      onClick={() => startMutation.mutate()}
      disabled={
        startMutation.isPending ||
        startMutation.isSuccess ||
        startMutation.isError
      }
      type="button"
    >
      {startMutation.isSuccess ? (
        "*/5"
      ) : (
        <span>{startMutation.isPending ? "..." : "*/5"}</span>
      )}
    </Button>
  );
}

export default App;
