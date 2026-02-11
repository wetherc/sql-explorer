import { createApp } from 'vue'
import { createPinia } from 'pinia'
import PrimeVue from 'primevue/config'
import ToastService from 'primevue/toastservice'
// import 'primevue/resources/themes/aura-dark-green/theme.css' // Removed
import 'primeflex/primeflex.css'
import 'primeicons/primeicons.css'

import App from './App.vue'
import router from './router'

import Aura from '@primeuix/themes/aura' // Added for PrimeVue 4.x theming

const app = createApp(App)

app.use(createPinia())
app.use(router)
app.use(PrimeVue, {
    theme: {
        preset: Aura,
        options: {
            darkModeSelector: 'system', // Use system dark mode preference
            cssLayer: {
                name: 'primevue',
                order: 'prime, primevue'
            }
        }
    }
})
app.use(ToastService)

app.mount('#app')
