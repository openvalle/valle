import type { TimelineDocumentView } from "@valle/player";
import type { TimelineDocument } from "@valle/engine/internal";

export type TimelineBand = "visual" | "audio" | "caption" | "adjustment";
export type VisualItem = TimelineDocument["document"]["visual"]["tracks"][number]["items"][number];
export type AudioItem = TimelineDocument["document"]["audio"]["tracks"][number]["items"][number];
export type CaptionItem = TimelineDocument["document"]["captions"]["tracks"][number]["items"][number];
export type AdjustmentItem = TimelineDocument["document"]["adjustments"][number];
export type TimelineSequenceItem = VisualItem | AudioItem | CaptionItem | AdjustmentItem;

export interface ProjectedSequenceItem<I extends TimelineSequenceItem = TimelineSequenceItem> {
  band: TimelineBand;
  trackId: string;
  trackIndex: number;
  itemIndex: number;
  item: I;
  timelinePath: string | null;
  startSeconds: number;
  durationSeconds: number;
  endSeconds: number;
  startFrame: number;
  durationFrames: number;
  endFrame: number;
  advancesCursor: boolean;
  sourceStartSeconds: number | null;
  sourceStartFrame: number | null;
  sourceRate: number | null;
  motionFrames?: TimelineDocumentView["sequences"][number]["items"][number]["motionFrames"];
}

export interface ProjectedSequenceTrack {
  band: TimelineBand;
  id: string;
  index: number;
  durationSeconds: number;
  items: ProjectedSequenceItem[];
}

/**
 * Join Rust/WASM's read-only Sequence projection back to canonical renderer items. Exact
 * accumulation and Number projection stay entirely in Rust.
 */
export function projectTimelineSequences(
  timeline: TimelineDocument,
  view: TimelineDocumentView,
): ProjectedSequenceTrack[] {
  return view.sequences.map((trackView) => {
    const track = trackView.band === "adjustment"
      ? null
      : trackAt(timeline, trackView.band, trackView.trackIndex);
    if (trackView.band !== "adjustment"
      && (!track || track.id !== trackView.trackId || track.items.length !== trackView.items.length)) {
      throw new Error(`Timeline ${trackView.band} projection does not match generated document`);
    }
    const items = trackView.items.map((itemView): ProjectedSequenceItem => {
      const item = trackView.band === "adjustment"
        ? timeline.document.adjustments.find((candidate) => candidate.id === itemView.itemId)
        : track?.items[itemView.itemIndex];
      if (!item || item.id !== itemView.itemId) {
        throw new Error(`Timeline item projection '${itemView.itemId}' is stale`);
      }
      return {
        band: trackView.band,
        trackId: trackView.trackId,
        trackIndex: trackView.trackIndex,
        itemIndex: itemView.itemIndex,
        item,
        timelinePath: itemView.timelinePath,
        startSeconds: itemView.startSeconds,
        durationSeconds: itemView.durationSeconds,
        endSeconds: itemView.endSeconds,
        startFrame: itemView.startFrame,
        durationFrames: itemView.durationFrames,
        endFrame: itemView.endFrame,
        advancesCursor: itemView.advancesCursor,
        sourceStartSeconds: itemView.sourceStartSeconds,
        sourceStartFrame: itemView.sourceStartFrame,
        sourceRate: itemView.sourceRate,
        motionFrames: itemView.motionFrames,
      };
    });
    return {
      band: trackView.band,
      id: trackView.trackId,
      index: trackView.trackIndex,
      durationSeconds: trackView.durationSeconds,
      items,
    };
  });
}

export function findProjectedItem(
  timeline: TimelineDocument,
  view: TimelineDocumentView,
  itemId: string,
): ProjectedSequenceItem | null {
  for (const track of projectTimelineSequences(timeline, view)) {
    const found = track.items.find((entry) => entry.item.id === itemId);
    if (found) return found;
  }
  return null;
}

function trackAt(
  timeline: TimelineDocument,
  band: TimelineBand,
  index: number,
): { id: string; items: TimelineSequenceItem[] } | undefined {
  if (band === "visual") return timeline.document.visual.tracks[index];
  if (band === "audio") return timeline.document.audio.tracks[index];
  if (band === "caption") return timeline.document.captions.tracks[index];
  return undefined;
}
