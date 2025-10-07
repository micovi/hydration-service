import "./App.css";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Client, cacheExchange, fetchExchange, gql } from "urql";
import { useQueryState, parseAsBoolean } from "nuqs";
import { CheckSquare, RefreshCwIcon } from "lucide-react";
import { Fragment } from "react";
import { toast } from "sonner";

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

const HB_URL = import.meta.env.VITE_HB_URL || "http://65.108.7.125:8734";
const BACKEND_URL = import.meta.env.VITE_BACKEND_URL || "http://localhost:8081";

export const STALE_TIME = 1000 * 60 * 5; // 5 minutes

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

interface ProcessRecord {
  id: string;
  name: string;
  type: string;
  latest_slot: string;
  latest_slot_timestamp: string;
  compute_at_slot: string;
  compute_at_slot_timestamp: string;
  taskId?: string;
}

function App() {
  const [debug] = useQueryState("debug", parseAsBoolean.withDefault(false));
  const queryClient = useQueryClient();

  const {
    data: processes,
    isLoading,
    error,
  } = useQuery<ProcessRecord[]>({
    queryKey: ["processes"],
    queryFn: async () => {
      const response = await fetch(BACKEND_URL);

      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }

      const data = (await response.json()) as { processes: ProcessRecord[] };

      return data.processes;
    },
  });

  const reloadData = () => {
    queryClient.invalidateQueries({ queryKey: ["computeAtSlot"] });
    queryClient.invalidateQueries({ queryKey: ["latestSlot"] });
  };

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
              <TableHead>
                <Button onClick={reloadData} type="button">
                  <RefreshCwIcon />
                </Button>
              </TableHead>
              {debug && <TableHead>Debug</TableHead>}
            </TableRow>
          </TableHeader>
          <TableBody>
            {processes.map((process) => (
              <ProcessRecordRow key={process.id} process={process} />
            ))}
          </TableBody>
        </Table>
      )}

      {/*  {debug && (
        <div className="my-4 p-4 border rounded">
          <h2 className="text-2xl mb-2">Add New Process</h2>
          <AddProcessForm />
        </div>
      )} */}
    </div>
  );
}

function ProcessRecordRow({ process }: { process: ProcessRecord }) {
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
        return edges[0].node.id;
      }

      return null;
    },
    staleTime: STALE_TIME * 5, // 25 minutes
  });

  /*   const {
    data: computeAtSlot,
    isLoading: isLoadingComputeAtSlot,
    isFetching: isReloadingComputeAtSlot,
    error: errorComputeAtSlot,
  } = useQuery<string>({
    queryKey: ["computeAtSlot", process.id],
    queryFn: async ({ signal }) => {
      const processInDb = await db.processes.get(process.id);

      const result = await fetch(
        `${HB_URL}/${process.id}~process@1.0/compute/at-slot`,
        { signal }
      );

      if (!result.ok) {
        throw new Error(`HTTP error! status: ${result.status}`);
      }

      const newSlot = await result.text();

      // Save to IndexedDB
      if (processInDb?.computeAtSlot !== Number(newSlot)) {
        await db.processes.upsert(process.id, {
          computeAtSlot: Number(newSlot),
        });
      }

      return newSlot;
    },
    staleTime: STALE_TIME,
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
    staleTime: STALE_TIME,
  });
 */

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

  const reloadSlotsMutation = useMutation({
    mutationFn: async () => {
      await fetch(`${BACKEND_URL}/compute/at-slot/${process.id}`);
      await fetch(`${BACKEND_URL}/slot/current/${process.id}`);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["processes"] });
    },
  });

  const stopTaskMutation = useMutation({
    mutationFn: async () => {
      const rs = await fetch(`${BACKEND_URL}/cron/stop/${process.id}`);

      const responseText = await rs.text();
      console.log(rs.status, responseText);

      return responseText;
    },
    onSuccess: async (responseText) => {
      queryClient.invalidateQueries({ queryKey: ["processes"] });
      console.log("Stopped 'once' process for:", process.id);
      toast.success(responseText);
    },
  });

  const reloadData = () => {
    queryClient.invalidateQueries({ queryKey: ["aoReserves", process.id] });
    queryClient.invalidateQueries({ queryKey: ["hbReserves", process.id] });
  };

  return (
    <Fragment key={process.id}>
      <TableRow>
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
        <TableCell className="font-mono">{process.latest_slot}</TableCell>
        <TableCell
          className={cn(
            "font-mono",
            process.latest_slot &&
              process.compute_at_slot &&
              (Number(process.compute_at_slot) < Number(process.latest_slot)
                ? "text-red-500"
                : "text-green-500")
          )}
        >
          {process.compute_at_slot}
        </TableCell>
        <TableCell>
          <Button
            onClick={() => {
              reloadSlotsMutation.mutate();
            }}
            type="button"
            disabled={reloadSlotsMutation.isPending}
          >
            <RefreshCwIcon
              className={cn(reloadSlotsMutation.isPending && "animate-spin")}
            />
          </Button>
        </TableCell>
        {debug && (
          <TableCell className="flex flex-row gap-1">
            {process.taskId ? (
              <Button
                className="font-mono"
                onClick={() => stopTaskMutation.mutate()}
                type="button"
                disabled={stopTaskMutation.isPending}
              >
                stop cron
              </Button>
            ) : (
              <>
                <OnceButton process={process} />
                <EveryButton process={process} />
              </>
            )}

            {checkpoint ? (
              <Button
                className="font-mono"
                onClick={() => loadCheckpointMutation.mutate()}
                type="button"
                disabled={loadCheckpointMutation.isPending}
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

      {process.type === "amm" && (
        <TableRow className="!p-0 !border-0">
          <TableCell
            colSpan={debug ? 7 : 6}
            className="bg-gray-50 !p-0 !border-0"
          >
            <DisplayReserves process={process} />
          </TableCell>
        </TableRow>
      )}
    </Fragment>
  );
}

function DisplayReserves({ process }: { process: ProcessRecord }) {
  const AO_CU_URL = "https://cu.ao-testnet.xyz";

  const {
    data: reserves,
    isLoading: isLoadingReserves,
    isFetching: isReloadingReserves,
  } = useQuery<Record<string, string>>({
    queryKey: ["aoReserves", process.id],
    queryFn: async () => {
      const payload = {
        Id: "1234",
        Target: process.id,
        Owner: "1234",
        Anchor: "0",
        Data: "1234",
        Tags: [
          { name: "Action", value: "Get-Reserves" },
          { name: "Data-Protocol", value: "ao" },
          { name: "Type", value: "Message" },
          { name: "Variant", value: "ao.TN.1" },
        ],
      };

      const url = `${AO_CU_URL}/dry-run?process-id=${encodeURIComponent(
        process.id
      )}`;

      const res = await fetch(url, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });

      if (!res.ok) {
        const text = await res.text();
        throw new Error(`AO dry-run failed: ${res.status} ${text}`);
      }

      const data = (await res.json()) as {
        Messages: { Tags: { name: string; value: string }[] | null }[];
      };
      const out: Record<string, string> = {};
      const messages = data?.Messages;
      if (Array.isArray(messages) && messages.length > 0) {
        const tags = messages[0]?.Tags ?? [];

        const skip = new Set([
          "Action",
          "Data-Protocol",
          "Type",
          "Variant",
          "Reference",
        ]);
        for (const tag of tags) {
          if (!tag?.name) continue;
          if (skip.has(tag.name)) continue;
          // Token addresses are 43 characters long
          if (tag.name.length === 43) {
            out[tag.name] = tag.value ?? "";
          }
        }
      }

      return out;
    },
    staleTime: STALE_TIME,
  });

  const {
    data: hbReserves,
    isLoading: isLoadingHbReserves,
    isFetching: isReloadingHbReserves,
  } = useQuery<Record<string, string>>({
    queryKey: ["hbReserves", process.id],
    queryFn: async () => {
      const res = await fetch(
        `${HB_URL}/${process.id}~process@1.0/now/reserves`
      );
      if (!res.ok) {
        const text = await res.text();
        throw new Error(`HB reserves failed: ${res.status} ${text}`);
      }

      const data = (await res.json()) as Record<string, string>;

      // Filter out non 43-character keys (not token addresses)
      for (const key of Object.keys(data)) {
        if (key.length !== 43) {
          delete data[key];
        }
      }

      return data;
    },
    staleTime: STALE_TIME,
  });

  const tokens = Array.from(
    new Set([...Object.keys(reserves ?? {}), ...Object.keys(hbReserves ?? {})])
  ).sort();

  return (
    <div>
      {tokens.length > 0 ? (
        <div className="overflow-x-auto opacity-60 hover:opacity-100">
          <table className="min-w-full text-sm !m-0 !border-0">
            <thead>
              <tr className="text-left">
                <th className="px-2 py-1 font-mono w-32">Token</th>
                <th className="px-2 py-1 font-mono">AO (DryRun)</th>
                <th className="px-2 py-1 font-mono">HB (/now)</th>
                <th className="px-2 py-1 font-mono">Difference</th>
              </tr>
            </thead>
            <tbody>
              {tokens.map((token) => {
                const aoValue = reserves?.[token] ?? "-";
                const hbValue = hbReserves?.[token] ?? "-";
                const mismatch =
                  aoValue !== "-" && hbValue !== "-" && aoValue !== hbValue;
                return (
                  <tr key={token} className={mismatch ? "text-red-500" : ""}>
                    <td className="px-2 py-1 font-mono">{token}</td>
                    <td className="px-2 py-1 font-mono">
                      {isLoadingReserves || isReloadingReserves
                        ? "loading..."
                        : aoValue}
                    </td>
                    <td className="px-2 py-1 font-mono">
                      {isLoadingHbReserves || isReloadingHbReserves
                        ? "loading..."
                        : hbValue}
                    </td>
                    <td className="px-2 py-1 font-mono">
                      {aoValue !== "-" && hbValue !== "-"
                        ? Number(hbValue) !== Number(aoValue)
                          ? "Yes"
                          : "No"
                        : "-"}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : (
        <p>No reserves found.</p>
      )}
    </div>
  );
}

function OnceButton({ process }: { process: ProcessRecord }) {
  const queryClient = useQueryClient();

  const startMutation = useMutation({
    mutationFn: async () => {
      const rs = await fetch(`${BACKEND_URL}/cron/once/${process.id}`);

      const responseId = await rs.text();

      console.log(rs.status, responseId);

      return responseId;
    },
    onSuccess: async (responseText) => {
      queryClient.invalidateQueries({ queryKey: ["processes"] });
      console.log(responseText);
      toast.success(`Task created with id: ${responseText}`);
    },
  });

  return (
    <Button
      className="font-mono"
      onClick={() => startMutation.mutate()}
      disabled={startMutation.isPending}
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

function EveryButton({ process }: { process: ProcessRecord }) {
  const queryClient = useQueryClient();
  const startMutation = useMutation({
    mutationFn: async () => {
      const rs = await fetch(`${BACKEND_URL}/cron/every/${process.id}`);

      const responseId = await rs.text();

      console.log(rs.status, responseId);

      return responseId;
    },
    onSuccess: async (responseText) => {
      queryClient.invalidateQueries({ queryKey: ["processes"] });
      console.log(responseText);
      toast.success(`Task created with id: ${responseText}`);
    },
  });
  return (
    <Button
      className="font-mono"
      onClick={() => startMutation.mutate()}
      disabled={startMutation.isPending}
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
