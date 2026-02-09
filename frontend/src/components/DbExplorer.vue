<script setup lang="ts">
// DbExplorer.vue
</script>

<template>
  <div class="db-explorer">
    <h3>Database Explorer</h3>
    <div class="tree-view">
      <ul>
        <li>
          <span class="tree-item">Databases</span>
          <ul>
            <li>
              <span class="tree-item">Database1</span>
              <ul>
                <li>
                  <span class="tree-item">Schemas</span>
                  <ul>
                    <li>
                      <span class="tree-item">dbo</span>
                      <ul>
                        <li>
                          <span class="tree-item">Tables</span>
                          <ul>
                            <li><span class="tree-item">Table1</span></li>
                            <li><span class="tree-item">Table2</span></li>
                          </ul>
                        </li>
                        <li>
                          <span class="tree-item">Views</span>
                          <ul>
                            <li><span class="tree-item">View1</span></li>
                          </ul>
                        </li>
                      </ul>
                    </li>
                    <li><span class="tree-item">guest</span></li>
                  </ul>
                </li>
                <li><span class="tree-item">Users</span></li>
              </ul>
            </li>
            <li><span class="tree-item">Database2</span></li>
          </ul>
        </li>
      </ul>
    </div>
  </div>
</template>

<style scoped>
.db-explorer {
  padding: 10px;
  background-color: #f8f9fa;
  height: 100%;
  overflow-y: auto;
}

h3 {
  color: #343a40;
  margin-bottom: 15px;
}

.tree-view ul {
  list-style-type: none;
  padding-left: 20px;
  margin: 0;
}

.tree-view ul li {
  margin-top: 5px;
}

.tree-item {
  cursor: pointer;
  padding: 2px 5px;
  border-radius: 3px;
}

.tree-item:hover {
  background-color: #e2e6ea;
}
</style>
