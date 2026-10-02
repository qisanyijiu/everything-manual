import {
  readDraftParts,
  readDraftReview,
  readDraftSpecs,
  readDraftSteps,
} from "../viewer/draft-view";

/** Display only the selected release's frozen edits; retain its supplier text for comparison.
 * Explicit per-kind fields keep IDs, evidence and geometry outside the text overlay.
 */
export function readReleaseKnowledge(knowledge: unknown, frozenReview: unknown) {
  const review = readDraftReview(frozenReview);
  const parts = readDraftParts(knowledge).map((original) => {
    const edit = review.entities[original.id]?.userEdited;
    return {
      ...original,
      name: edit?.name ?? original.name,
      description: edit?.description ?? original.description,
      original,
      hasUserEdit: edit?.name !== undefined || edit?.description !== undefined,
    };
  });
  const steps = readDraftSteps(knowledge).map((original) => {
    const edit = review.entities[original.id]?.userEdited;
    return {
      ...original,
      title: edit?.title ?? original.title,
      orderedActions: edit?.orderedActions ?? original.orderedActions,
      original,
      hasUserEdit: edit?.title !== undefined || edit?.orderedActions !== undefined,
    };
  });
  const specs = readDraftSpecs(knowledge).map((original) => {
    const edit = review.entities[original.id]?.userEdited;
    return {
      ...original,
      label: edit?.label ?? original.label,
      value: edit?.value ?? original.value,
      original,
      hasUserEdit: edit?.label !== undefined || edit?.value !== undefined,
    };
  });
  return { parts, steps, specs, review };
}
