import { LitElement, html } from "lit";

export class ValleConsoleApp extends LitElement {
  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected render() {
    return html`
<header class="topbar">
  <span class="brand">
    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m4 5 8 15L20 5M9 5l3 6 3-6"/></svg>
    valle
    <span class="sub">Console</span>
  </span>
  <nav class="tabs" role="tablist">
    <button class="tab" id="tabBtnLibrary" role="tab" data-tab="library" aria-selected="true">Assets</button>
    <button class="tab" id="tabBtnProjects" role="tab" data-tab="projects" aria-selected="false">Projects</button>
  </nav>
  <span id="loading">Loading…</span>
</header>


  <section class="panel" id="panelLibrary" role="tabpanel">
    <div class="toolbar">
      <input type="search" id="libSearch" placeholder="Search titles, annotations, transcripts…" />
      <label><input type="checkbox" id="libRemoved" />Show removed</label>
      <button class="button" id="libRefresh" type="button">Refresh</button>
    </div>
    <div class="dash" id="libDash" hidden></div>
    <div class="note" id="libNote" hidden></div>
    <div class="grid" id="libGrid"></div>
    <div class="empty" id="libEmpty" hidden>
      The library is empty. Add assets with <code>valle assets add</code>.
    </div>
  </section>

  <section class="panel" id="panelProjects" role="tabpanel" hidden>
    <div class="toolbar">
      <span class="muted">Project attached to this host</span>
      <button class="button quiet" id="projRefresh" type="button">Refresh</button>
    </div>
    <div class="note" id="projNote" hidden></div>
    <div class="list" id="projList"></div>
    <div class="empty" id="projEmpty" hidden>No project attached. Run valle project studio &lt;ID&gt; to open a project.</div>
  </section>

  <aside id="drawer" hidden>
    <div class="drawer-head">
      <b id="dTitle"></b>
      <button class="icon-button" id="dClose" type="button" title="Close (Esc)" aria-label="Close">×</button>
    </div>
    <div id="dPreview"></div>
    <dl id="dMeta"></dl>
    <div id="dTags"></div>
    <h4>Edit</h4>
    <div class="action-row">
      <input type="text" id="dTitleEdit" placeholder="Title" />
      <button class="button" id="dTitleSave" type="button">Save</button>
    </div>
    <div class="action-row" id="dSubkindRow" hidden>
      <select id="dSubkind">
        <option value="">subkind…</option>
        <option value="music">music</option>
        <option value="sfx">sfx</option>
      </select>
      <button class="button" id="dSubkindSave" type="button">Save</button>
    </div>
    <div class="action-row">
      <input type="text" id="dTagInput" placeholder="Add tag" />
      <button class="button" id="dTagAdd" type="button">Add</button>
    </div>
    <h4>Annotations</h4>
    <ul id="dAnnotations"></ul>
    <div class="action-row">
      <input type="text" id="dAnnText" placeholder="Annotation text" />
      <input type="text" id="dAnnRange" class="narrow" placeholder="83 or 83-102 (s)" />
      <button class="button" id="dAnnAdd" type="button">Add</button>
    </div>
    <h4>Analysis</h4>
    <ul id="dAnalysis"></ul>
    <div class="action-row">
      <input type="text" id="dAnWith" placeholder="shots,asr" value="shots,asr" />
      <input type="text" id="dAnBudget" class="narrow" placeholder="Budget (CNY)" />
      <button class="button" id="dAnRun" type="button">Run</button>
    </div>
    <div class="note" id="dProgress" hidden></div>
    <div class="action-row"><button class="button danger" id="dRm" type="button">Remove, retain annotations</button></div>
  </aside>
  <div id="errbox" hidden><b>Console error</b><br /><span id="errtext"></span></div>


    `;
  }
}

if (!customElements.get("valle-console-app")) {
  customElements.define("valle-console-app", ValleConsoleApp);
}

declare global {
  interface HTMLElementTagNameMap {
    "valle-console-app": ValleConsoleApp;
  }
}
