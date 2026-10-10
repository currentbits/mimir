import { Composition, staticFile } from "remotion";
import { loadFont } from "@remotion/fonts";
import { Film } from "./Film";
import T from "./timeline.json";

const FPS = 60;

loadFont({ family: "Junicode", url: staticFile("fonts/junicode-roman.woff2"), weight: "300 700" });
loadFont({ family: "Junicode", url: staticFile("fonts/junicode-italic.woff2"), weight: "300 700", style: "italic" });

export const RemotionRoot: React.FC = () => (
  <Composition
    id="Mimir"
    component={Film}
    durationInFrames={Math.round(((T.bars * 4 * 60) / T.bpm) * FPS)}
    fps={FPS}
    width={1920}
    height={1080}
  />
);
