// Live (non-server) state: the daemon connection.
import { create } from "zustand";
import type { ConnectionStatus } from "./bindings";

interface LiveState {
  connection: ConnectionStatus;
  setConnection: (c: ConnectionStatus) => void;
}

export const useLive = create<LiveState>((set) => ({
  connection: { state: "connecting" },
  setConnection: (connection) => set({ connection }),
}));
