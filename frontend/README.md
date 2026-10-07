# SQL Explorer frontend

This folder contains the user interface of SQL Explorer. It uses Vue 3,
Vuetify, Pinia and the Monaco editor, and Vite builds it. The Tauri backend in
`../backend` shows the build in its window.

Run these commands from the root of the repository. The root `README.md` gives
the full set.

```sh
pnpm dev                          # start the application with hot reloading
pnpm --filter frontend test:unit  # run the unit tests
pnpm --filter frontend test:coverage  # run the unit tests with the coverage gate
pnpm --filter frontend lint       # run ESLint
pnpm --filter frontend typecheck  # run vue-tsc
pnpm --filter frontend format     # format every file with Prettier
```

The `src/` folder has these parts:

```
components/   The views and their tests
layouts/      The shell of the application
lib/          The calls to the backend and the pure helpers
plugins/      The set-up of Vuetify, Monaco and the icons
stores/       The Pinia stores that keep the state of the interface
types/        The types of the data that the backend sends
```

The user guide is in `../docs/guide/`. The build bundles it into the
application.
