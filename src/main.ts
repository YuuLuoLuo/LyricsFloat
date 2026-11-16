import { createApp } from 'vue'
import './styles/theme.css'
import { currentLabel } from './core/bridge'

async function boot() {
  const label = await currentLabel()

  if (label === 'settings') {
    document.title = 'LyricsFloat 设置'
    document.body.classList.add('win-settings')
    const { default: SettingsApp } = await import('./views/SettingsApp.vue')
    createApp(SettingsApp).mount('#app')
  } else {
    document.body.classList.add('win-board')
    const { default: BoardApp } = await import('./App.vue')
    createApp(BoardApp).mount('#app')
  }
}

void boot()
