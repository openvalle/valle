import { LitElement, html } from "lit";

export class ValleConsoleApp extends LitElement {
  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected render() {
    return html`
      <header class="topbar">
        <span class="brand"><span class="dot"></span>Valle Console<span class="sub">Management</span></span>
        <nav class="tabs" role="tablist">
          <button class="tab" id="tabBtnLibrary" role="tab" data-tab="library" aria-selected="true">Asset library</button>
          <button class="tab" id="tabBtnProjects" role="tab" data-tab="projects" aria-selected="false">Projects</button>
        </nav>
        <span id="loading">Loading…</span>
      </header>

      <section class="panel" id="panelLibrary" role="tabpanel">
        <div class="toolbar">
          <input type="search" id="libSearch" placeholder="Search titles, annotations, transcripts, or entities" />
          <label><input type="checkbox" id="libRemoved" />Show removed</label>
          <button id="libRefresh">Refresh</button>
        </div>
        <div class="dash" id="libDash" hidden></div>
        <div class="note" id="libNote" hidden></div>
        <div class="grid" id="libGrid"></div>
        <div class="empty" id="libEmpty" hidden>The library is empty. Add assets with <code>valle assets add</code>.</div>
      </section>

      <section class="panel" id="panelProjects" role="tabpanel" hidden>
        <div class="toolbar">
          <input type="text" id="projName" placeholder="New project name (optional)" />
          <button id="projCreate">Create project</button>
          <button id="projRefresh">Refresh</button>
        </div>
        <div class="note" id="projNote" hidden></div>
        <div id="projList"></div>
        <div class="empty" id="projEmpty" hidden>No projects yet</div>
      </section>

      <aside id="drawer" hidden>
        <div class="drawer-head">
          <b id="dTitle"></b>
          <button id="dClose" title="Close (Esc)">×</button>
        </div>
        <div id="dPreview"></div>
        <dl id="dMeta"></dl>
        <div id="dTags"></div>
        <h4>Edit</h4>
        <div class="action-row">
          <input type="text" id="dTitleEdit" placeholder="Title" />
          <button id="dTitleSave">Save</button>
        </div>
        <div class="action-row" id="dSubkindRow" hidden>
          <select id="dSubkind">
            <option value="">subkind…</option>
            <option value="music">music</option>
            <option value="sfx">sfx</option>
          </select>
          <button id="dSubkindSave">Save</button>
        </div>
        <div class="action-row">
          <input type="text" id="dTagInput" placeholder="Add tag" />
          <button id="dTagAdd">Add</button>
        </div>
        <h4>Annotations</h4>
        <ul id="dAnnotations"></ul>
        <div class="action-row">
          <input type="text" id="dAnnText" placeholder="Annotation text" />
          <input type="text" id="dAnnRange" class="narrow" placeholder="83 or 83-102 (seconds, optional)" />
          <button id="dAnnAdd">Add</button>
        </div>
        <h4>Analysis</h4>
        <ul id="dAnalysis"></ul>
        <div class="action-row">
          <input type="text" id="dAnWith" placeholder="shots,asr" value="shots,asr" />
          <input type="text" id="dAnBudget" class="narrow" placeholder="Budget in CNY (optional)" />
          <button id="dAnRun">Run</button>
        </div>
        <div class="note" id="dProgress" hidden></div>
        <div class="action-row">
          <button id="dRm" class="danger">Remove, retain annotations</button>
        </div>
      </aside>

      <div id="errbox" hidden><b>console error</b><br /><span id="errtext"></span></div>
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
