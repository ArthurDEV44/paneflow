import type { Component } from "solid-js";

const App: Component = () => {
  return (
    <div class="app">
      <div class="sidebar">
        <h2>PaneFlow</h2>
        <p>Workspaces will appear here</p>
      </div>
      <div class="main-area">
        <p>Terminal panes will render here</p>
      </div>
    </div>
  );
};

export default App;
