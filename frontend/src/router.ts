import { useEffect, useState } from 'react'

export type AppRoute =
  | { name: 'chat' }
  | { name: 'task'; taskId: string }

export function useRoute(): AppRoute {
  const [route, setRoute] = useState(readRoute)

  useEffect(() => {
    const handleRouteChange = () => setRoute(readRoute())
    window.addEventListener('popstate', handleRouteChange)
    return () => window.removeEventListener('popstate', handleRouteChange)
  }, [])

  return route
}

export function navigateTo(path: string) {
  window.history.pushState(null, '', path)
  window.dispatchEvent(new PopStateEvent('popstate'))
}

function readRoute(): AppRoute {
  const taskMatch = window.location.pathname.match(/^\/tasks\/([^/]+)$/)
  if (taskMatch?.[1]) {
    return { name: 'task', taskId: decodeURIComponent(taskMatch[1]) }
  }

  return { name: 'chat' }
}
