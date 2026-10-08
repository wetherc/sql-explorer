// The styles of the component library come first. The library puts its rules
// in cascade layers, and the first file that names a layer fixes the order of
// the layers. A component file loaded first names its own layer before the
// reset layer, and the reset rule `button, input { font: inherit }` then
// removes the text size and the weight of every button and field.
import vuetify from './plugins/vuetify'
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import { configureMonacoEnvironment } from './plugins/monaco'
import './style.css'

configureMonacoEnvironment()

createApp(App).use(createPinia()).use(vuetify).mount('#app')
