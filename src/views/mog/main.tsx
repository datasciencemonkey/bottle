/** V2 MOG view entry. The MOG ID comes from the `mog-<id>` window label. */
import './mog.css';
import { mount } from '../mount';
import { ownMogId } from '../view-ipc';
import { MogView } from './MogView';

const mogId = ownMogId('mog');
mount(mogId ? <MogView mogId={mogId} /> : <p role="alert">Unknown MOG window</p>);
