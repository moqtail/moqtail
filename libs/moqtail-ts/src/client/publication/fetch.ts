import {
  Fetch,
  FetchHeader,
  FetchHeaderType,
  FetchType,
  InternalError,
  Location,
  MoqtailError,
  MoqtObject,
} from '@/model'
import { MoqtailClient } from '../client'
import { Track } from '../track/track'
import { SubscribePublication } from './subscribe'

// TODO: Use group order
// TODO: Use fetch parameters
export class FetchPublication {
  readonly #requestId: bigint
  readonly #track: Track
  readonly #startLocation: Location
  readonly #endLocation: Location
  readonly #msg: Fetch
  readonly #client: MoqtailClient
  #stream: WritableStream | undefined
  #writer: WritableStreamDefaultWriter | undefined
  #objects: MoqtObject[] | undefined
  #isCanceled = false

  constructor(client: MoqtailClient, track: Track, fetchRequest: Fetch) {
    this.#client = client
    this.#requestId = fetchRequest.requestId
    this.#track = track
    this.#msg = fetchRequest
    let joiningRequest: SubscribePublication | FetchPublication | undefined
    switch (this.#msg.typeAndProps.type) {
      case FetchType.StandAlone:
        // TODO: Tie up fetch type and relevant props as {type: 1, props: standAlone} | {type: 2, props: joining} | {type: 3, props: joining}
        this.#startLocation = this.#msg.typeAndProps.props.startLocation
        this.#endLocation = this.#msg.typeAndProps.props.endLocation
        break

      case FetchType.Relative:
        joiningRequest = client.publications.get(this.#msg.typeAndProps.props.joiningRequestId)
        if (!(joiningRequest instanceof SubscribePublication))
          throw new InternalError('FetchPublication.constructor', 'No subscription for the joining request id')
        if (!joiningRequest.latestLocation)
          throw new InternalError('FetchPublication.constructor', 'joiningRequest.largestLocation does not exist')
        this.#startLocation = new Location(
          joiningRequest.latestLocation.group - this.#msg.typeAndProps.props.joiningStart,
          0n,
        )
        this.#endLocation = joiningRequest.latestLocation
        break

      case FetchType.Absolute:
        joiningRequest = client.publications.get(this.#msg.typeAndProps.props.joiningRequestId)
        if (!(joiningRequest instanceof SubscribePublication))
          throw new InternalError('FetchPublication.constructor', 'No subscription for the joining request id')
        if (!joiningRequest.latestLocation)
          throw new InternalError('FetchPublication.constructor', 'joiningRequest.largestLocation does not exist')
        this.#startLocation = new Location(this.#msg.typeAndProps.props.joiningStart, 0n)
        this.#endLocation = joiningRequest.latestLocation
        break
    }
    this.publish()
  }

  cancel() {
    this.#isCanceled = true
  }

  async publish(): Promise<void> {
    if (this.#isCanceled) return
    if (!this.#track.contentSource.getObjectRange)
      throw new MoqtailError('FetchPublication.publish, Track does not support fetch')
    try {
      this.#objects = await this.#track.contentSource.getObjectRange(this.#startLocation, this.#endLocation)
      // TODO: Calculate and use stream priority from subscriber priority from the msg + publisher priority from the track
      this.#stream = await this.#client.webTransport.createUnidirectionalStream()
      this.#writer = this.#stream.getWriter()
      const header = new FetchHeader(FetchHeaderType.Type0x05, this.#requestId)
      await this.#writer.write(header)
      for (const obj of this.#objects) {
        if (this.#isCanceled) {
          await this.#writer.abort('Fetch cancelled during publish')
          this.#client.publications.delete(this.#requestId)
          return
        }
        await this.#writer.write(obj.tryIntoFetchObject().serialize().toUint8Array())
      }
      await this.#writer.close()
      this.#client.publications.delete(this.#requestId)
    } catch (error: unknown) {
      await this.#writer?.abort('Fetch failed during publish')
      const message = error instanceof Error ? error.message : String(error)
      throw new InternalError('FetchPublication.publish', `Failed to publish: ${message}`)
    }
  }
}
