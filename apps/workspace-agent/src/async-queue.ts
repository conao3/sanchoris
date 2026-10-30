export class AsyncQueue<T> implements AsyncIterable<T> {
  private readonly items: T[] = []
  private waiter: ((result: IteratorResult<T>) => void) | null = null
  private ended = false

  push(item: T): void {
    if (this.ended) throw new Error("Queue has ended")
    if (this.waiter) {
      const waiter = this.waiter
      this.waiter = null
      waiter({ value: item, done: false })
      return
    }
    this.items.push(item)
  }

  end(): void {
    this.ended = true
    if (this.waiter) {
      const waiter = this.waiter
      this.waiter = null
      waiter({ value: undefined, done: true })
    }
  }

  [Symbol.asyncIterator](): AsyncIterator<T> {
    return {
      next: () => {
        const item = this.items.shift()
        if (item !== undefined) return Promise.resolve({ value: item, done: false })
        if (this.ended) return Promise.resolve({ value: undefined, done: true })
        return new Promise((resolve) => {
          this.waiter = resolve
        })
      },
    }
  }
}
