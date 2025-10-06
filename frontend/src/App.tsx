import "./App.css";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Client, cacheExchange, fetchExchange, gql } from "urql";
import { useQueryState, parseAsBoolean } from "nuqs";

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

interface Process {
  name: string;
  id: string;
  type: string;
}

export const HB_URL = "https://hb.zoao.dev";

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
  } = useQuery<Process[]>({
    queryKey: ["processes"],
    queryFn: () => fetch("/hydration-service/processes.json").then((res) => res.json()),
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
        return edges[0].node.id;
      }

      return null;
    },
    staleTime: 1000 * 60 * 5, // 5 minutes
  });

  const {
    data: computeAtSlot,
    isLoading: isLoadingComputeAtSlot,
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

  const onceMutation = useMutation({
    mutationFn: async () => {
      const rs = await fetch(
        `${HB_URL}/~cron@1.0/once?cron-path=/${process.id}~process@1.0/now`
      );

      console.log(rs.status, await rs.text());
    },
    onSuccess: () => {
      // Invalidate queries to refresh data
      queryClient.invalidateQueries({
        queryKey: ["computeAtSlot", process.id],
      });
      queryClient.invalidateQueries({ queryKey: ["latestSlot", process.id] });
    },
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
      {debug && (
        <>
          <TableCell>
            {checkpoint ? (
              <Button
                className="font-mono"
                onClick={() => loadCheckpointMutation.mutate()}
                type="button"
              >
                checkpoint
              </Button>
            ) : isLoadingCheckpoint ? (
              "Loading..."
            ) : (
              errorCheckpoint?.message
            )}
          </TableCell>
          <TableCell>
            <Button
              className="font-mono"
              onClick={() => onceMutation.mutate()}
              disabled={
                onceMutation.isPending ||
                onceMutation.isSuccess ||
                onceMutation.isError
              }
              type="button"
            >
              {onceMutation.isSuccess ? (
                "Triggered"
              ) : (
                <span>{onceMutation.isPending ? "Triggering..." : "once"}</span>
              )}
            </Button>
          </TableCell>
        </>
      )}
    </TableRow>
  );
}

export default App;
