/**
 * On (re)attach a node writes the ring-buffer snapshot first, then applies only the live
 * chunks the snapshot didn't already cover. This one comparison is the guard that stops a
 * reload from double-writing or dropping terminal output.
 */
export function pastSnapshot(seq: number, lastSeq: number): boolean {
  return seq > lastSeq;
}
