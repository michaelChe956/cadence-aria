import { create } from "zustand";
import { getProviderStatus, recheckProviders } from "../api/client";
import type {
  ProviderHealthEntry,
  ProviderHealthResponse,
  ProviderHealthStateStatus,
  RealProviderName,
} from "../api/types";

export type ProviderAvailabilityLoadStatus =
  | "idle"
  | "loading"
  | "loaded"
  | "error";

export type ProviderAvailabilityRecheckStatus =
  | "idle"
  | "rechecking"
  | "error";

export type ProviderHealthByName = Partial<
  Record<RealProviderName, ProviderHealthEntry>
>;

export type ProviderAvailabilityState = {
  snapshot: ProviderHealthResponse | null;
  loadStatus: ProviderAvailabilityLoadStatus;
  recheckStatus: ProviderAvailabilityRecheckStatus;
  error: string | null;
  generation: number | null;
  stateStatus: ProviderHealthStateStatus | null;
  stateError: string | null;
  realWorkflowBlocked: boolean;
  testProviderEnabled: boolean;
  providers: ProviderHealthByName;
};

export type ProviderAvailabilityActions = {
  load: () => Promise<void>;
  recheck: () => Promise<void>;
  reset: () => void;
};

type ProviderAvailabilityStore = ProviderAvailabilityState &
  ProviderAvailabilityActions;

const initialState: ProviderAvailabilityState = {
  snapshot: null,
  loadStatus: "idle",
  recheckStatus: "idle",
  error: null,
  generation: null,
  stateStatus: null,
  stateError: null,
  realWorkflowBlocked: true,
  testProviderEnabled: false,
  providers: {},
};

let recheckInFlight: Promise<void> | null = null;
let resetEpoch = 0;

function providerIndex(entries: ProviderHealthEntry[]): ProviderHealthByName {
  return entries.reduce<ProviderHealthByName>((providers, entry) => {
    providers[entry.provider] = entry;
    return providers;
  }, {});
}

function applySnapshot(
  state: ProviderAvailabilityState,
  snapshot: ProviderHealthResponse,
  transition: Partial<ProviderAvailabilityState>,
): Partial<ProviderAvailabilityState> {
  if (state.generation !== null && snapshot.generation < state.generation) {
    return transition;
  }

  return {
    ...transition,
    snapshot,
    generation: snapshot.generation,
    stateStatus: snapshot.state_status,
    stateError: snapshot.state_error,
    realWorkflowBlocked: snapshot.real_workflow_blocked,
    testProviderEnabled: snapshot.test_provider_enabled,
    providers: providerIndex(snapshot.providers),
  };
}

function requestErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return "provider availability request failed";
}

export const useProviderAvailabilityStore = create<ProviderAvailabilityStore>(
  (set) => ({
    ...initialState,
    load: () => {
      const requestEpoch = resetEpoch;
      set({ loadStatus: "loading", error: null });
      return getProviderStatus()
        .then((snapshot) => {
          if (requestEpoch !== resetEpoch) {
            return;
          }
          set((state) =>
            applySnapshot(state, snapshot, {
              loadStatus: "loaded",
              error: null,
            }),
          );
        })
        .catch((error: unknown) => {
          if (requestEpoch !== resetEpoch) {
            return;
          }
          set({
            loadStatus: "error",
            error: requestErrorMessage(error),
          });
        });
    },
    recheck: () => {
      if (recheckInFlight) {
        return recheckInFlight;
      }

      const requestEpoch = resetEpoch;
      set({ recheckStatus: "rechecking", error: null });
      const request = (async () => {
        try {
          const snapshot = await recheckProviders();
          if (requestEpoch !== resetEpoch) {
            return;
          }
          set((state) =>
            applySnapshot(state, snapshot, {
              recheckStatus: "idle",
              error: null,
            }),
          );
        } catch (error: unknown) {
          if (requestEpoch !== resetEpoch) {
            return;
          }
          set({
            recheckStatus: "error",
            error: requestErrorMessage(error),
          });
        }
      })();
      recheckInFlight = request;
      void request.finally(() => {
        if (recheckInFlight === request) {
          recheckInFlight = null;
        }
      });
      return request;
    },
    reset: () => {
      resetEpoch += 1;
      recheckInFlight = null;
      set({ ...initialState, providers: {} });
    },
  }),
);
