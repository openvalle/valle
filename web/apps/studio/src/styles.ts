import { css } from "lit";

// Shared surfaces and resizable panels for the Studio workspace.
export const styles = css`
  :host {
    --bg: #090a0c;
    --panel: #121317;
    --well: #0c0d10;
    --control: #1e2026;
    --hover: #282b33;
    --line: #22252c;
    --edge: #343843;
    --text: #e9ebf0;
    --muted: #8d94a3;
    --accent: #7eaaff;
    --action: #3478f0;
    --mono: ui-monospace, "SF Mono", Menlo, Monaco, Consolas, monospace;
    --left: 236px;
    --right: 340px;
    --bottom: 264px;
    display: grid;
    grid-template-rows: 56px minmax(140px, 1fr) 8px minmax(100px, var(--bottom)) 26px;
    height: 100dvh;
    min-height: 420px;
    padding: 0 10px;
    box-sizing: border-box;
    overflow: hidden;
    color-scheme: dark;
    background: var(--bg);
    color: var(--text);
    font:
      12px/1.5 system-ui,
      sans-serif;
    -webkit-font-smoothing: antialiased;
  }
  :host([mode="motion"]) {
    grid-template-rows: 56px minmax(140px, 1fr) 0 0 26px;
  }
  :host([collapsed]) {
    grid-template-rows: 56px minmax(140px, 1fr) 8px 40px 26px;
  }
  :host([focused]) {
    grid-template-rows: 56px minmax(140px, 1fr) 0 0 26px;
  }
  :host([focused]) .workspace {
    grid-template-columns: minmax(0, 1fr);
  }
  :host([focused]) .left,
  :host([focused]) .right,
  :host([focused]) .split,
  :host([focused]) .bottom,
  :host([focused]) .horizontal {
    display: none;
  }
  :host([inspector-hidden]) .workspace {
    grid-template-columns: var(--left) 8px minmax(0, 1fr);
  }
  :host([inspector-hidden]) .right,
  :host([inspector-hidden]) .right-split {
    display: none;
  }
  :host([focused][inspector-hidden]) .workspace {
    grid-template-columns: minmax(0, 1fr);
  }
  *,
  *::before,
  *::after {
    box-sizing: border-box;
  }
  [hidden] {
    display: none !important;
  }
  * {
    scrollbar-width: thin;
    scrollbar-color: var(--edge) transparent;
  }
  button,
  input,
  select,
  textarea {
    font: inherit;
    color: inherit;
  }
  button,
  a,
  input,
  select,
  textarea {
    outline-offset: 3px;
  }
  :focus-visible {
    outline: 2px solid var(--accent);
  }
  button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 4px 10px;
    background: var(--control);
    cursor: pointer;
    white-space: nowrap;
    transition:
      background-color 120ms,
      border-color 120ms,
      color 120ms;
  }
  button:hover:not(:disabled) {
    background: var(--hover);
  }
  button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  button.active,
  button[aria-pressed="true"] {
    color: var(--accent);
    background: #7eaaff1f;
  }
  button:not(.quiet, .primary, .icon) {
    border-color: #ffffff12;
  }
  button.primary {
    background: #7eaaff24;
    border-color: #7eaaff4d;
    color: #d5e3ff;
  }
  button.primary:hover:not(:disabled) {
    background: #7eaaff38;
    border-color: #7eaaff73;
  }
  button.quiet {
    background: transparent;
  }
  button.icon {
    width: 30px;
    height: 30px;
    padding: 6px;
    flex-shrink: 0;
  }
  svg {
    flex-shrink: 0;
  }
  a {
    color: var(--accent);
    text-decoration: none;
  }
  input:not([type="range"]):not([type="checkbox"]),
  select {
    min-width: 0;
    width: 100%;
    border: 1px solid var(--line);
    background: var(--well);
    border-radius: 6px;
    padding: 5px 8px;
    transition: border-color 120ms;
  }
  input:not([type="range"]):not([type="checkbox"]):hover,
  select:hover {
    border-color: var(--edge);
  }
  input:not([type="range"]):not([type="checkbox"]):focus-visible,
  select:focus-visible,
  textarea:focus-visible {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px #7eaaff26;
  }
  input::placeholder {
    color: #5c6270;
  }
  select {
    appearance: none;
    padding-right: 24px;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%238d94a3' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='m7 10 5 5 5-5'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 5px center;
    background-size: 14px;
  }
  input[type="range"] {
    min-width: 40px;
    accent-color: var(--accent);
    cursor: pointer;
  }
  input[type="checkbox"] {
    appearance: none;
    position: relative;
    justify-self: end;
    width: 28px;
    height: 16px;
    margin: 0;
    border-radius: 999px;
    background: #2b2f39;
    cursor: pointer;
    transition: background-color 140ms;
  }
  input[type="checkbox"]::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #c9ced8;
    transition: transform 140ms;
  }
  input[type="checkbox"]:checked {
    background: var(--action);
  }
  input[type="checkbox"]:checked::after {
    background: white;
    transform: translateX(12px);
  }
  input[type="number"] {
    font-variant-numeric: tabular-nums;
  }
  h1,
  h2,
  h3,
  p {
    margin: 0;
  }
  h3 {
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
  }
  small,
  .muted {
    color: var(--muted);
  }
  .spacer {
    flex: 1;
  }
  .divider {
    height: 18px;
    width: 1px;
    background: var(--line);
    margin: 0 5px;
  }
  .topbar {
    display: flex;
    gap: 8px;
    align-items: center;
    min-width: 0;
  }
  .brand {
    display: flex;
    align-items: center;
    margin: 0 4px;
  }
  .logo {
    width: 28px;
    height: 24px;
    object-fit: contain;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
    font-size: 13px;
  }
  .badge {
    color: var(--muted);
    background: var(--control);
    border-radius: 4px;
    padding: 2px 6px;
    font-size: 9px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .save-state {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--muted);
    font-size: 11px;
    white-space: nowrap;
  }
  .save-state.dirty {
    color: #f2bb68;
  }
  .topbar select {
    width: auto;
    max-width: 180px;
  }
  .workspace {
    display: grid;
    grid-template-columns: var(--left) 8px minmax(0, 1fr) 8px var(--right);
    min-height: 0;
  }
  .panel {
    min-width: 0;
    min-height: 0;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 10px;
    box-shadow: inset 0 1px 0 #ffffff08;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .panel-head {
    display: flex;
    align-items: center;
    min-height: 42px;
    padding: 0 13px;
    gap: 8px;
    border-bottom: 1px solid var(--line);
    flex-shrink: 0;
  }
  .panel-head strong {
    font-size: 12px;
    font-weight: 550;
  }
  .panel-head small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .scroll {
    flex: 1;
    overflow: auto;
    min-height: 0;
    padding: 14px;
  }
  .tabs {
    display: flex;
    align-items: stretch;
    border-bottom: 1px solid var(--line);
    padding: 6px;
    gap: 2px;
    flex-shrink: 0;
  }
  .tabs button {
    flex: 1;
    border: 0;
    border-radius: 6px;
    padding: 5px;
    background: none;
    color: var(--muted);
    font-size: 11px;
  }
  .tabs button:hover:not(:disabled) {
    background: none;
    color: var(--text);
  }
  .tabs button[aria-selected="true"] {
    color: var(--text);
    background: var(--control);
    font-weight: 550;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 7px;
    margin: 10px 10px 8px;
    color: var(--muted);
  }
  .search input {
    font-size: 11px;
  }
  .files {
    padding: 0 7px 12px;
  }
  details > summary {
    cursor: pointer;
    color: var(--muted);
    padding: 8px 4px;
    font-size: 11px;
  }
  .file {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 8px;
    width: 100%;
    padding: 5px 8px;
    background: none;
    border: 0;
    border-radius: 6px;
    text-align: left;
  }
  .file .filename {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .file small {
    font-size: 10px;
  }
  .file svg {
    color: var(--muted);
  }
  .file.open {
    background: #7eaaff1f;
    color: #cfe0ff;
  }
  .file.open svg {
    color: var(--accent);
  }
  .dot {
    color: #f2bb68;
    font-size: 9px;
  }
  .empty {
    color: var(--muted);
    text-align: center;
    padding: 32px 16px;
    line-height: 1.8;
  }
  .empty svg {
    width: 28px;
    height: 28px;
    margin-bottom: 12px;
    opacity: 0.6;
  }
  .empty strong {
    display: block;
    color: var(--text);
    font-size: 12px;
    font-weight: 500;
    margin-bottom: 6px;
  }
  .media-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
    padding: 0 12px 12px;
  }
  .asset {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    padding: 0;
    overflow: hidden;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--well);
  }
  .asset:hover:not(:disabled) {
    background: var(--well);
    border-color: var(--edge);
  }
  .thumbnail {
    aspect-ratio: 16 / 9;
    display: grid;
    place-items: center;
    color: var(--muted);
    background: #202229;
  }
  .thumbnail img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .asset.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .thumbnail .wave {
    width: 100%;
    height: 70%;
    fill: #4f9d83;
  }
  .asset span:last-child {
    text-align: left;
    padding: 6px;
    font-size: 10px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .stage {
    position: relative;
  }
  .canvas {
    flex: 1;
    min-height: 0;
    min-width: 0;
    padding: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: #08090b;
    background-image: radial-gradient(#ffffff09 1px, transparent 1px);
    background-size: 14px 14px;
  }
  .canvas canvas {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .error {
    position: absolute;
    z-index: 1;
    inset: 12px 12px auto;
    padding: 12px;
    border: 1px solid #8d4949;
    background: #321d23;
    border-radius: 6px;
    max-height: 50%;
    overflow: auto;
  }
  .error pre {
    margin: 0 0 10px;
    white-space: pre-wrap;
    font: 11px/1.6 var(--mono);
  }
  .playback-rate {
    min-width: 8ch;
    text-align: right;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    color: var(--muted);
    white-space: nowrap;
  }
  .canvas-size {
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    padding: 0 6px 0 8px;
    border-left: 1px solid var(--line);
  }
  .transport {
    border-top: 1px solid var(--line);
    padding: 6px 12px 9px;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: center;
    gap: 5px;
  }
  .scrubber {
    grid-column: 1 / -1;
    position: relative;
    height: 26px;
    margin-bottom: 3px;
    border-radius: 5px;
  }
  .scrub-rail {
    position: absolute;
    inset: 11px 6px;
    height: 4px;
    border-radius: 999px;
    background: #ffffff19;
    pointer-events: none;
  }
  .scrub-fill {
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, #4e83ec, #a8caff);
    transform: scaleX(var(--scrub-progress));
    transform-origin: left;
  }
  .scrub-thumb {
    position: absolute;
    top: -4px;
    left: 0;
    width: 100%;
    height: 12px;
    transform: translateX(calc(var(--scrub-progress) * 100%));
  }
  .scrub-thumb::after {
    content: "";
    display: block;
    width: 12px;
    height: 12px;
    margin-left: -6px;
    border-radius: 50%;
    background: #edf4ff;
    box-shadow:
      0 1px 4px #0008,
      0 0 0 3px #86b4ff12;
  }
  .scrubber[data-playing="true"] .scrub-fill {
    transition: transform var(--scrub-step) linear;
  }
  .scrubber[data-playing="true"] .scrub-thumb {
    transition: transform var(--scrub-step) linear;
  }
  .scrubber:hover .scrub-thumb::after,
  .scrubber:focus-within .scrub-thumb::after {
    box-shadow:
      0 1px 4px #0008,
      0 0 0 5px #86b4ff24;
  }
  .scrubber:has(:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .scrubber[data-disabled="true"] {
    opacity: 0.35;
  }
  .scrubber .scrub {
    position: absolute;
    inset: 0;
    appearance: none;
    width: 100%;
    height: 100%;
    margin: 0;
    padding: 0;
    background: transparent;
    outline: none;
    touch-action: none;
  }
  .scrub::-webkit-slider-runnable-track {
    height: 4px;
    background: transparent;
  }
  .scrub::-webkit-slider-thumb {
    appearance: none;
    width: 12px;
    height: 26px;
    margin-top: -11px;
    background: transparent;
  }
  .scrub::-moz-range-track {
    height: 4px;
    background: transparent;
  }
  .scrub::-moz-range-progress {
    background: transparent;
  }
  .scrub::-moz-range-thumb {
    width: 12px;
    height: 26px;
    border: 0;
    background: transparent;
  }
  .time {
    font: 11px var(--mono);
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .time .frame {
    margin-left: 10px;
    padding: 1px 6px;
    border: 1px solid var(--line);
    border-radius: 4px;
    color: var(--muted);
    font-size: 10px;
  }
  .transport .time {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .transport .playback {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .transport .playback .icon:not(.quiet),
  .media-controls > .icon:not(.quiet) {
    border-radius: 50%;
    background: #ffffff1a;
    color: var(--text);
  }
  .transport .playback .icon:not(.quiet) {
    width: 34px;
    height: 34px;
  }
  .transport .playback .icon:not(.quiet):hover:not(:disabled),
  .media-controls > .icon:not(.quiet):hover {
    background: #ffffff2e;
  }
  .playback .icon:not(.quiet) svg,
  .media-controls > .icon:not(.quiet) svg {
    fill: currentColor;
  }
  .transport .toggle-group {
    display: flex;
    align-items: center;
    justify-self: end;
    min-width: 0;
  }
  .toast {
    position: fixed;
    z-index: 10;
    left: 50%;
    bottom: 40px;
    transform: translateX(-50%);
    max-width: min(560px, calc(100vw - 32px));
    padding: 10px 14px;
    border: 1px solid #8d4949;
    border-radius: 8px;
    background: #321d23;
    color: #ffd9d9;
    box-shadow: 0 10px 40px #0009;
    overflow-wrap: anywhere;
  }
  .split {
    cursor: col-resize;
    touch-action: none;
    position: relative;
  }
  .horizontal {
    cursor: row-resize;
    touch-action: none;
    position: relative;
  }
  .split:hover::after,
  .split:focus-visible::after {
    content: "";
    position: absolute;
    inset: 25% 3px;
    background: var(--accent);
    border-radius: 2px;
  }
  .horizontal:hover::after,
  .horizontal:focus-visible::after {
    content: "";
    position: absolute;
    inset: 3px 35%;
    background: var(--accent);
    border-radius: 2px;
  }
  .bottom {
    min-height: 0;
  }
  .bottom timeline-view {
    flex: 1;
    min-height: 0;
  }
  .timeline-tools {
    min-height: 38px;
    padding: 0 8px 0 13px;
    gap: 4px;
  }
  .timeline-tools button.quiet:not(.icon) {
    padding: 4px 9px;
    color: var(--muted);
    font-size: 11px;
  }
  .timeline-tools button.quiet:not(.icon):hover {
    color: var(--text);
  }
  .timeline-tools input {
    width: 90px;
  }
  .source {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .source-name {
    padding: 10px 13px;
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
  }
  .editor {
    display: flex;
    min-height: 0;
    flex: 1;
    border-top: 1px solid var(--line);
    background: #111216;
  }
  .gutter {
    flex: 0 0 42px;
    margin: 0;
    padding: 12px 8px;
    text-align: right;
    color: #5f6570;
    overflow: hidden;
    user-select: none;
    font: 12px/20px var(--mono);
  }
  textarea {
    display: block;
    width: 100%;
    min-height: 120px;
    resize: vertical;
    border: 1px solid var(--line);
    border-radius: 5px;
    background: #111216;
    padding: 10px;
    font: 12px/20px var(--mono);
    tab-size: 2;
  }
  .editor textarea {
    flex: 1;
    min-width: 0;
    min-height: 0;
    border: 0;
    border-radius: 0;
    resize: none;
    padding: 12px 10px 12px 0;
    outline: none;
  }
  .source-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 12px;
    font-size: 10px;
    color: var(--muted);
    border-top: 1px solid var(--line);
  }
  .section {
    padding-bottom: 14px;
    margin-bottom: 14px;
    border-bottom: 1px solid var(--line);
  }
  .section:last-child {
    border: 0;
  }
  .section h3 {
    margin-bottom: 12px;
    text-transform: uppercase;
    letter-spacing: 0.7px;
    font-size: 10px;
  }
  .field {
    display: grid;
    grid-template-columns: minmax(70px, 1fr) minmax(0, 1.3fr);
    align-items: center;
    gap: 10px;
    margin: 7px 0;
    font-size: 11px;
    color: var(--muted);
  }
  .field > span:last-child {
    color: var(--text);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .field input,
  .field select {
    color: var(--text);
  }
  .field .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }
  .field.stacked {
    grid-template-columns: minmax(0, 1fr);
    gap: 6px;
  }
  .field.stacked textarea {
    min-height: 0;
    font: inherit;
    color: var(--text);
  }
  .section:has(> details:only-child) {
    padding-bottom: 0;
  }
  .banner {
    padding: 12px;
    margin: 10px;
    border: 1px solid #735633;
    background: #30271b;
    border-radius: 6px;
    font-size: 11px;
    overflow-wrap: anywhere;
  }
  .banner .actions {
    margin-top: 8px;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .media-preview {
    width: 100%;
    max-height: 240px;
    object-fit: contain;
    border-radius: 5px;
    margin-bottom: 14px;
    background: #101115;
  }
  .section > .actions {
    margin-top: 12px;
  }
  a.button {
    padding: 4px 10px;
    border: 1px solid #ffffff12;
    border-radius: 6px;
    color: var(--text);
  }
  a.button:hover {
    background: var(--hover);
  }
  .media-player {
    margin-bottom: 14px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: #101115;
    overflow: hidden;
  }
  .media-player .media-preview {
    display: block;
    margin: 0;
    border-radius: 0;
    background: #000;
    cursor: pointer;
  }
  .media-player audio {
    display: none;
  }
  .media-wave {
    position: relative;
    display: grid;
    place-items: center;
    height: 84px;
    margin: 12px 12px 4px;
    color: var(--muted);
    cursor: pointer;
  }
  .media-wave svg[viewBox$=" 2"] {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    fill: #34514a;
  }
  .media-wave svg.played {
    fill: #6fd0ad;
  }
  .media-controls {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
  }
  .media-controls .scrubber {
    flex: 1;
    min-width: 0;
    margin: 0;
  }
  .media-controls .time {
    font-size: 10px;
    white-space: nowrap;
  }
  .media-controls .time .muted {
    color: var(--muted);
  }
  .render-card {
    --progress-color: #8cb7ff;
    padding: 16px;
    border: 1px solid var(--line);
    border-radius: 10px;
    margin-bottom: 10px;
    background: linear-gradient(145deg, #202632, #181b22 70%);
  }
  .render-heading,
  .render-label,
  .render-detail {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .render-heading,
  .render-detail {
    justify-content: space-between;
  }
  .render-label {
    min-width: 0;
  }
  .render-label strong {
    font-size: 11px;
    font-weight: 550;
    line-height: 1.4;
  }
  .render-mark {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    flex: 0 0 16px;
    color: var(--progress-color);
    border: 1.5px solid #8cb7ff30;
    border-top-color: var(--progress-color);
    border-radius: 50%;
    animation: render-spin 900ms linear infinite;
  }
  .render-percent {
    min-width: 4ch;
    text-align: right;
    color: var(--progress-color);
    font-size: 20px;
    font-weight: 550;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.6px;
  }
  .render-track {
    height: 6px;
    margin: 14px 0 10px;
    border-radius: 999px;
    background: #ffffff0b;
    overflow: hidden;
  }
  .render-fill {
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, #4e83ec, var(--progress-color));
    transform: scaleX(var(--render-progress));
    transform-origin: left;
    transition: transform 320ms cubic-bezier(0.2, 0.7, 0.3, 1);
  }
  .render-detail {
    min-height: 16px;
    font-size: 10px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .render-card[data-state="preparing"] .render-fill {
    width: 30%;
    animation: render-wait 1.6s ease-in-out infinite;
  }
  .render-card[data-state="complete"] {
    --progress-color: #86d9b3;
  }
  .render-card[data-state="error"] {
    --progress-color: #ee9595;
  }
  .render-card:is([data-state="complete"], [data-state="error"]) .render-mark {
    border: 0;
    animation: none;
  }
  .render-card:is([data-state="complete"], [data-state="error"]) .render-fill {
    background: var(--progress-color);
  }
  @keyframes render-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes render-wait {
    from {
      transform: translateX(-110%);
    }
    to {
      transform: translateX(340%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .scrubber[data-playing="true"] .scrub-fill,
    .scrubber[data-playing="true"] .scrub-thumb,
    .render-fill {
      transition: none;
    }
    .render-mark {
      animation: none;
    }
    .render-card[data-state="preparing"] .render-fill {
      animation: none;
      transform: translateX(110%);
      opacity: 0.6;
    }
  }
  .render-card .render-path {
    margin: 12px 0 0;
    font-size: 10px;
    color: var(--muted);
    overflow-wrap: anywhere;
  }
  .render-open {
    display: inline-flex;
    align-items: center;
    margin-top: 12px;
    padding: 6px 10px;
    border: 1px solid #86d9b329;
    border-radius: 6px;
    background: #86d9b30a;
    color: var(--progress-color);
    font-size: 11px;
    text-decoration: none;
  }
  .render-open:hover {
    background: #86d9b31a;
  }
  .statusbar {
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 10px;
    color: var(--muted);
    overflow: hidden;
  }
  .statusbar .message {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  dialog {
    width: min(460px, calc(100vw - 32px));
    border: 1px solid #3a3e48;
    border-radius: 14px;
    padding: 0;
    background: var(--panel);
    color: var(--text);
    box-shadow:
      0 24px 80px #000b,
      inset 0 1px 0 #ffffff0d;
  }
  dialog::backdrop {
    background: #0008;
    backdrop-filter: blur(4px);
  }
  dialog .dialog-body {
    padding: 24px;
  }
  dialog h2 {
    font-size: 20px;
    font-weight: 550;
    letter-spacing: -0.5px;
  }
  dialog p {
    color: var(--muted);
    margin: 6px 0 20px;
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  dialog .dialog-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 14px 24px;
    border-top: 1px solid var(--line);
    background: #1b1d22;
  }
  .home {
    grid-row: 2 / -1;
    max-width: 760px;
    width: 100%;
    margin: 0 auto;
    padding: 56px 24px;
    overflow: auto;
  }
  .hero {
    display: flex;
    align-items: center;
    gap: 20px;
    margin-bottom: 28px;
  }
  .hero img {
    width: 56px;
    height: 48px;
    object-fit: contain;
  }
  .home h1 {
    font-size: 30px;
    letter-spacing: -1px;
    font-weight: 550;
  }
  .hero p {
    color: var(--muted);
    margin-top: 4px;
    font-size: 13px;
  }
  .home-actions {
    display: flex;
    gap: 10px;
    margin-bottom: 36px;
  }
  .home-actions button {
    padding: 9px 14px;
  }
  .home h3 {
    margin-bottom: 10px;
    text-transform: uppercase;
    letter-spacing: 0.7px;
    font-size: 10px;
  }
  .notice {
    margin-bottom: 24px;
    padding: 10px 12px;
    border: 1px solid #735633;
    border-radius: 6px;
    background: #30271b;
    overflow-wrap: anywhere;
  }
  .projects {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 10px;
    margin-bottom: 28px;
  }
  .project {
    justify-content: flex-start;
    gap: 12px;
    padding: 12px;
    border: 1px solid var(--line);
    border-radius: 9px;
    background: var(--panel);
    text-align: left;
  }
  .project:hover:not(:disabled) {
    border-color: #464c5a;
  }
  .project-mark {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    border-radius: 8px;
    background: linear-gradient(145deg, #27406b, #1b2233);
    color: var(--accent);
  }
  .project-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .project-text strong,
  .project-text small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .project-text strong {
    font-size: 13px;
    font-weight: 550;
  }
  .by-path form {
    display: flex;
    gap: 8px;
    max-width: 480px;
  }
  .by-path button {
    border-color: var(--line);
  }
  dialog .field.stacked {
    margin: 14px 0 0;
  }
  dialog .actions input {
    flex: 1;
  }
  .topbar {
    grid-row: 1;
  }
  .workspace {
    grid-row: 2;
  }
  .horizontal {
    grid-row: 3;
  }
  .bottom {
    grid-row: 4;
  }
  .statusbar {
    grid-row: 5;
  }
  :host([mode="motion"]) .horizontal,
  :host([collapsed]) .horizontal {
    visibility: hidden;
  }
  @media (max-width: 1100px) {
    :host {
      --left: 190px;
      --right: 280px;
    }
    .save-state,
    .badge {
      display: none;
    }
    .canvas {
      padding: 12px;
    }
  }
  @media (max-width: 850px) {
    .workspace {
      grid-template-columns: minmax(0, 1fr) 8px min(300px, 42%);
    }
    :host([inspector-hidden]) .workspace {
      grid-template-columns: minmax(0, 1fr);
    }
    .left,
    .left-split {
      display: none;
    }
    .topbar .name {
      flex: 1;
    }
    .topbar .brand {
      margin-right: 0;
    }
    .topbar .save-state,
    .topbar .badge,
    .topbar .divider,
    .canvas-size,
    .time .frame {
      display: none;
    }
  }
`;
