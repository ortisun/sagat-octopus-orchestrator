const loadedAt = Date.now();

export async function stamped(): Promise<number> {
  return loadedAt;
}
