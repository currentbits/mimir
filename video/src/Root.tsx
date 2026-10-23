import { Composition, staticFile } from "remotion";
import { loadFont } from "@remotion/fonts";
import { Film } from "./Film";
import { Guide } from "./Guide";
import { Ad, Ad15, CUT } from "./Ad";
import A from "./ad.json";
import G from "./guide.json";
import T from "./timeline.json";

const FPS = 60;

loadFont({ family: "Junicode", url: staticFile("fonts/junicode-roman.woff2"), weight: "300 700" });
loadFont({ family: "Junicode", url: staticFile("fonts/junicode-italic.woff2"), weight: "300 700", style: "italic" });

const frames = (t: { bars: number; bpm: number }) => Math.round(((t.bars * 4 * 60) / t.bpm) * FPS);

export const RemotionRoot: React.FC = () => (
  <>
    <Composition id="Mimir" component={Film} durationInFrames={frames(T)} fps={FPS} width={1920} height={1080} />
    <Composition id="Ad" component={Ad} durationInFrames={frames(A)} fps={FPS} width={1920} height={1080} />
    <Composition id="AdUpright" component={Ad} durationInFrames={frames(A)} fps={FPS} width={1080} height={1920} />
    <Composition id="Ad15" component={Ad15} durationInFrames={frames({ bpm: A.bpm, bars: CUT.reduce((n, [a, b]) => n + b - a, 0) })} fps={FPS} width={1920} height={1080} />
    <Composition id="Ad15Upright" component={Ad15} durationInFrames={frames({ bpm: A.bpm, bars: CUT.reduce((n, [a, b]) => n + b - a, 0) })} fps={FPS} width={1080} height={1920} />
    <Composition id="Guide" component={Guide} durationInFrames={frames(G)} fps={FPS} width={1920} height={1080} />
  </>
);
