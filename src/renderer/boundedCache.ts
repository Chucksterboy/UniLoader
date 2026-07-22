export function rememberBoundedSetValue<T>(
  values: Set<T>,
  value: T,
  maxEntries: number
): void {
  values.delete(value);
  values.add(value);

  while (values.size > Math.max(0, maxEntries)) {
    const oldest = values.values().next().value as T | undefined;
    if (oldest === undefined) {
      break;
    }
    values.delete(oldest);
  }
}
