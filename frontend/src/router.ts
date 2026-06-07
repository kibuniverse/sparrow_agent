import { createRouter } from '@tanstack/react-router'
import { routeTree } from './routeTree.gen'

const router = createRouter({
  routeTree,
  basepath: '/sparrow_agent',
})

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}

export { router }
