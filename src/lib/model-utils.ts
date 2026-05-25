import type { ModelDto } from "$lib/tauri-bridge";

/** GGUF and cloud models that can be loaded from the chat header. */
export function isLoadableModel(m: ModelDto): boolean {
  return m.weightsFormat === "gguf" || m.weightsFormat === "cloud";
}

export function loadableModels(models: ModelDto[]): ModelDto[] {
  return models.filter(isLoadableModel);
}

/** First candidate id that refers to a loadable model, else first loadable model. */
export function resolveLoadableModelId(
  models: ModelDto[],
  candidates: (string | null | undefined)[],
): string {
  for (const id of candidates) {
    if (!id) continue;
    const m = models.find((x) => x.id === id);
    if (m && isLoadableModel(m)) return id;
  }
  return loadableModels(models)[0]?.id ?? "";
}
