import { useEffect, useState } from 'react'

export type AppRoute =
  | { name: 'chat' }
  | { name: 'task'; taskId: string }
  | { name: 'trace-file'; fileName: string }
  | { name: 'replay'; fileName: string }

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
  const traceFileMatch = window.location.pathname.match(/^\/trace-files\/([^/]+)$/)
  if (traceFileMatch?.[1]) {
    return { name: 'trace-file', fileName: decodeURIComponent(traceFileMatch[1]) }
  }

  const replayMatch = window.location.pathname.match(/^\/replay\/([^/]+)$/)
  if (replayMatch?.[1]) {
    return { name: 'replay', fileName: decodeURIComponent(replayMatch[1]) }
  }

  const taskMatch = window.location.pathname.match(/^\/tasks\/([^/]+)$/)
  if (taskMatch?.[1]) {
    return { name: 'task', taskId: decodeURIComponent(taskMatch[1]) }
  }

  return { name: 'chat' }
}
