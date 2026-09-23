// Coalesce refresh requests and wait for every loader before starting another snapshot.
export function createSnapshotRefresh(loaders: () => Array<() => Promise<void>>): () => Promise<void> {
  let inFlight: Promise<void> | null = null;
  let queued = false;
  return () => {
    if (inFlight !== null) {
      queued = true;
      return inFlight;
    }
    const run = async () => {
      let failure: PromiseRejectedResult | undefined;
      do {
        queued = false;
        const results = await Promise.allSettled(loaders().map(load => Promise.resolve().then(load)));
        failure = results.find(result => result.status === "rejected");
      } while (queued);
      if (failure !== undefined) throw failure.reason;
    };
    inFlight = run().finally(() => { inFlight = null; });
    return inFlight;
  };
}
