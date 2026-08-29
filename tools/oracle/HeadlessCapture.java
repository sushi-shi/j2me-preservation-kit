/*
 * HeadlessCapture -- a headless capture frontend for FreeJ2ME-Plus.
 *
 * This is NOT part of FreeJ2ME. It is a thin driver that uses the emulator's
 * own public API (org.recompile.mobile.Mobile / MobilePlatform) in place of the
 * AWT frontend, so that frames can be captured without an X display.
 *
 * It does not modify emulator behaviour: it sets the same Mobile.* switches the
 * AWT/Libretro frontends set, installs a painter that copies the front buffer,
 * and injects MIDP key codes through MobilePlatform.keyPressed/keyReleased.
 *
 * -------------------------------------------------------------------------
 * Provenance
 * -------------------------------------------------------------------------
 *
 * Adapted, near-verbatim, from stalker-mobile's
 * tools/transliteration/java-me/HeadlessCapture.java (the 3D Stalker port's
 * frame oracle). A 2D J2ME port's game is pure 2D
 * LCDUI (Graphics), so the M3G-only switches and the Stalker-specific game-state
 * snapshot were dropped, and the RNG-by-reflection seeding was generalised to
 * scan every loaded game class for a static java.util.Random rather than naming
 * one obfuscated class. Everything else -- painter->PNG on `shot`, Nokia keycode
 * injection, the deterministic clock and gated frame stepping -- is the same.
 *
 * ---------------------------------------------------------------------------
 * The route script format (shared with the future Rust port)
 * ---------------------------------------------------------------------------
 *
 * One command per line; `#` starts a comment. Every command accepts trailing
 * `key=value` tokens, and **any key this driver does not know is ignored**.
 * That is deliberate: the same file is meant to be read by more than one
 * consumer -- this driver, the future `<game> --script` runner, and
 * `tools/oracle/compare_frames.py` -- and each takes the tokens it needs.
 *
 *   wait <ms>                 sleep; `frames=<n>` is the port's unit
 *   tap <KEY> [hold] [settle] press, hold ms, release, settle ms
 *   hold <KEY> [ms]           press and leave pressed
 *   release <KEY> [ms]        release
 *   seed <n>                  reseed every loaded game java.util.Random in place
 *   fps <n>                   switch to deterministic gated frame stepping
 *   shot <label> [k=v ...]    write <label>.png
 *   echo <text>
 *
 * Durations are milliseconds here because the emulator is normally paced by
 * wall clock. The port reads `frames=`/`settle_frames=` instead. A route that
 * must act on the immediately following Java game frame may opt into
 * `java_frames=` (and `java_settle_frames=` for a tap's released half). After
 * an `fps` directive, the driver gates those paints and advances FreeJ2ME's
 * substituted game clock by the command's exact millisecond budget. This still
 * sends ordinary key events; it makes both update count and elapsed game time
 * independent of host speed.
 */

import org.recompile.mobile.Mobile;
import org.recompile.mobile.MobilePlatform;
import org.recompile.mobile.MIDletEnhancements;
import org.recompile.mobile.PlatformImage;

import javax.imageio.ImageIO;
import javax.microedition.lcdui.Display;
import java.awt.image.BufferedImage;
import java.io.BufferedReader;
import java.io.File;
import java.io.FileReader;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;

public class HeadlessCapture
{
	static MobilePlatform platform;
	static File outDir;
	static String jarPath;
	static volatile long frameCount = 0;
	static final Object FRAME_GATE = new Object();
	static volatile boolean frameStepping = false;
	static long permittedThroughFrame = Long.MAX_VALUE;
	static long startedAtMillis = 0;
	static PrintWriter shotLog;
	static final Map<String, Integer> KEYS = new HashMap<String, Integer>();

	static
	{
		/* Nokia / MIDP key codes as FreeJ2ME's Mobile class defines them. */
		KEYS.put("UP", -1);
		KEYS.put("DOWN", -2);
		KEYS.put("LEFT", -3);
		KEYS.put("RIGHT", -4);
		KEYS.put("FIRE", -5);
		KEYS.put("SOFT1", -6);
		KEYS.put("SOFT2", -7);
		KEYS.put("SEND", -10);
		KEYS.put("END", -11);
		KEYS.put("STAR", 42);
		KEYS.put("POUND", 35);
		for (int i = 0; i <= 9; i++) { KEYS.put("NUM" + i, 48 + i); }
	}

	public static void main(String[] args) throws Exception
	{
		String jar = null, script = null, out = null;
		int w = 240, h = 320;
		int logLevel = 3;
		int backlight = 0;
		boolean sound = false;

		for (int i = 0; i < args.length; i++)
		{
			if (args[i].equals("--jar")) { jar = args[++i]; }
			else if (args[i].equals("--width")) { w = Integer.parseInt(args[++i]); }
			else if (args[i].equals("--height")) { h = Integer.parseInt(args[++i]); }
			else if (args[i].equals("--out")) { out = args[++i]; }
			else if (args[i].equals("--script")) { script = args[++i]; }
			else if (args[i].equals("--log")) { logLevel = Integer.parseInt(args[++i]); }
			else if (args[i].equals("--backlight")) { backlight = Integer.parseInt(args[++i]); }
			else if (args[i].equals("--sound")) { sound = Integer.parseInt(args[++i]) != 0; }
			else { throw new IllegalArgumentException("unknown arg " + args[i]); }
		}
		if (jar == null || out == null || script == null)
		{
			System.err.println("usage: HeadlessCapture --jar F --out DIR --script F [--width W --height H --log N]");
			System.exit(2);
		}

		jarPath = jar;
		outDir = new File(out);
		outDir.mkdirs();
		shotLog = new PrintWriter(new File(outDir, "shots.tsv"), "UTF-8");
		shotLog.println("label\tframes\telapsed_ms");

		Mobile.clearOldLog();
		Mobile.minLogLevel = (byte) logLevel;
		Mobile.sound = sound;                 /* off by default: no audio device in this sandbox */
		Mobile.lcdWidth = w;
		Mobile.lcdHeight = h;
		Mobile.limitFPS = 0;                  /* run free; the script paces by wall clock */
		/*
		 * Mobile.maskIndex defaults to 1 (a green "LCD backlight" tint) in the raw static
		 * initialiser, and PlatformGraphics.flushGraphics ANDs lcdMaskColors[maskIndex] into
		 * every blitted pixel. The AWT/Libretro frontends never see that default because
		 * Config seeds "backlightcolor" = "Disabled" (index 0). This driver does not use
		 * Config, so it must set the same thing explicitly or every capture comes out green.
		 */
		Mobile.maskIndex = backlight;
		Mobile.renderLCDMask = backlight != 0;

		platform = new MobilePlatform(w, h);
		Mobile.setPlatform(platform, new Runnable() { public void run() { } });

		Mobile.getPlatform().setPainter(new Runnable()
		{
			public void run()
			{
				synchronized (FRAME_GATE)
				{
					frameCount++;
					FRAME_GATE.notifyAll();
					while (frameStepping && frameCount >= permittedThroughFrame)
					{
						try { FRAME_GATE.wait(); }
						catch (InterruptedException e) { Thread.currentThread().interrupt(); return; }
					}
				}
			}
		});

		String url = new File(jar).getAbsoluteFile().toURI().toString();
		System.out.println("[drv] loading " + url);
		if (!platform.load(url))
		{
			System.out.println("[drv] LOAD FAILED");
			System.exit(1);
		}
		System.out.println("[drv] loaded; suite=" + platform.loader.suitename + " vendor=" + platform.loader.vendorname);

		startedAtMillis = System.currentTimeMillis();

		/* runJar() blocks inside MIDlet startApp for some MIDlets, so run it on its own thread. */
		Thread midletThread = new Thread(new Runnable()
		{
			public void run()
			{
				try { platform.runJar(); }
				catch (Throwable t) { System.out.println("[drv] runJar threw: " + t); t.printStackTrace(); }
			}
		}, "midlet-main");
		midletThread.setDaemon(true);
		midletThread.start();

		try { runScript(script); }
		catch (Throwable t)
		{
			System.err.println("[drv] route failed: " + t);
			t.printStackTrace();
			shotLog.flush();
			System.err.flush();
			Runtime.getRuntime().halt(1);
		}

		shotLog.flush();
		shotLog.close();
		System.out.println("[drv] done; frames=" + frameCount);
		System.out.flush();
		Runtime.getRuntime().halt(0);
	}

	/** A parsed command: the verb, its positional arguments, and its `k=v` tokens. */
	static final class Command
	{
		String verb;
		final List<String> positional = new ArrayList<String>();
		final Map<String, String> named = new HashMap<String, String>();

		/** A named token, else the positional at `index`, else `fallback`. */
		long millis(String key, int index, long fallback)
		{
			if (named.containsKey(key)) { return Long.parseLong(named.get(key)); }
			if (positional.size() > index) { return Long.parseLong(positional.get(index)); }
			return fallback;
		}
	}

	static Command parse(String line)
	{
		Command command = new Command();
		String[] tokens = line.split("\\s+");
		command.verb = tokens[0];
		for (int i = 1; i < tokens.length; i++)
		{
			int equals = tokens[i].indexOf('=');
			/*
			 * A `k=v` token is metadata for one of the consumers. Keys this driver
			 * does not know -- `frames=`, `layer=`, `port=` -- are kept in the map
			 * and simply never read, rather than rejected: the format is shared, and
			 * a route file must not have to be split per consumer.
			 */
			if (equals > 0) { command.named.put(tokens[i].substring(0, equals), tokens[i].substring(equals + 1)); }
			else { command.positional.add(tokens[i]); }
		}
		return command;
	}

	static void enableFrameStepping() throws Exception
	{
		MIDletEnhancements.enableDeterministicClock();
		Display.setDeterministicInputDispatch(true);
		Mobile.limitFPS = 0;
		synchronized (FRAME_GATE)
		{
			frameStepping = true;
			permittedThroughFrame = frameCount;
			long target = frameCount + 1;
			long deadline = System.currentTimeMillis() + 30000;
			while (frameCount < target)
			{
				long remaining = deadline - System.currentTimeMillis();
				if (remaining <= 0) { throw new IllegalStateException("timed out establishing frame gate"); }
				FRAME_GATE.wait(Math.min(remaining, 100));
			}
		}
	}

	static void stepFrames(long count, long millis) throws Exception
	{
		if (count < 0) { throw new IllegalArgumentException("frame count must be non-negative"); }
		if (count == 0)
		{
			MIDletEnhancements.advanceDeterministicClock(millis);
			return;
		}
		long share = millis / count;
		for (long index = 0; index < count; index++)
		{
			long step = index + 1 == count ? millis - share * (count - 1) : share;
			MIDletEnhancements.advanceDeterministicClock(step);
			synchronized (FRAME_GATE)
			{
				long target = frameCount + 1;
				permittedThroughFrame = target;
				FRAME_GATE.notifyAll();
				long deadline = System.currentTimeMillis() + 30000;
				while (frameCount < target)
				{
					long remaining = deadline - System.currentTimeMillis();
					if (remaining <= 0) { throw new IllegalStateException("timed out stepping a Java frame"); }
					FRAME_GATE.wait(Math.min(remaining, 100));
				}
			}
		}
	}

	/** Wait by gated reference paints when requested, otherwise by wall-clock time. */
	static void waitFor(Command command, String frameKey, String millisKey,
		int millisIndex, long fallbackMillis) throws Exception
	{
		if (!command.named.containsKey(frameKey))
		{
			Thread.sleep(command.millis(millisKey, millisIndex, fallbackMillis));
			return;
		}
		long count = Long.parseLong(command.named.get(frameKey));
		if (count < 0) { throw new IllegalArgumentException(frameKey + " must be non-negative"); }
		if (frameStepping)
		{
			long budget = command.named.containsKey("java_ms")
				? Long.parseLong(command.named.get("java_ms"))
				: command.millis(millisKey, millisIndex, fallbackMillis);
			stepFrames(count, budget);
			return;
		}
		long target = frameCount + count;
		long deadline = System.currentTimeMillis() + 30000;
		while (frameCount < target)
		{
			if (System.currentTimeMillis() >= deadline)
			{
				throw new IllegalStateException("timed out waiting for " + count + " Java frames");
			}
			Thread.sleep(1);
		}
	}

	/**
	 * Reseed every static java.util.Random the game has loaded, in place.
	 *
	 * Stalker named one obfuscated class; this game (obfuscated too) keeps its
	 * RNGs as `public static Random a` in more than one class, so rather than
	 * hard-code a name this walks the archive's class list, asks the game's own
	 * loader which of them are already loaded, and reseeds any static Random it
	 * finds. Game-agnostic: an archive with no such field simply reseeds nothing.
	 */
	static void seedGameRandom(long seed) throws Exception
	{
		Method findLoaded = ClassLoader.class.getDeclaredMethod("findLoadedClass", String.class);
		findLoaded.setAccessible(true);
		List<String> reseeded = new ArrayList<String>();
		for (String name : gameClassNames())
		{
			Class<?> owner = (Class<?>) findLoaded.invoke(platform.loader, name);
			if (owner == null) { continue; }
			for (Field field : owner.getDeclaredFields())
			{
				if (field.getType() != java.util.Random.class) { continue; }
				if (!java.lang.reflect.Modifier.isStatic(field.getModifiers())) { continue; }
				field.setAccessible(true);
				java.util.Random random = (java.util.Random) field.get(null);
				if (random != null) { random.setSeed(seed); reseeded.add(name + "." + field.getName()); }
			}
		}
		System.out.println("[drv] seed " + seed + " -> reseeded " + reseeded.size()
			+ " game Random field(s) " + reseeded);
	}

	/** Every class name in the archive, dotted (so `findLoadedClass` can probe each). */
	static List<String> gameClassNames() throws Exception
	{
		List<String> names = new ArrayList<String>();
		java.io.FileInputStream in = new java.io.FileInputStream(jarPath);
		ZipInputStream zip = new ZipInputStream(in);
		ZipEntry entry;
		while ((entry = zip.getNextEntry()) != null)
		{
			String path = entry.getName();
			if (path.endsWith(".class"))
			{
				names.add(path.substring(0, path.length() - 6).replace('/', '.'));
			}
		}
		zip.close();
		return names;
	}

	/** Deliver the input callback through Display, at this exact route boundary. */
	static void dispatchQueuedInputs() throws Exception
	{
		if (frameStepping)
		{
			if (Mobile.getDisplay() == null) { throw new IllegalStateException("Display is not initialized"); }
			Mobile.getDisplay().processInputEventsNow();
		}
	}

	static void runScript(String path) throws Exception
	{
		List<String> lines = new ArrayList<String>();
		BufferedReader r = new BufferedReader(new FileReader(path));
		String l;
		while ((l = r.readLine()) != null) { lines.add(l); }
		r.close();

		for (String raw : lines)
		{
			String line = raw.trim();
			if (line.isEmpty() || line.startsWith("#")) { continue; }

			if (line.startsWith("echo"))
			{
				System.out.println("[drv] " + line.substring(4).trim() + "  (frames=" + frameCount + ")");
				continue;
			}

			Command command = parse(line);
			String verb = command.verb;

			if (verb.equals("wait"))
			{
				waitFor(command, "java_frames", "ms", 0, 0);
			}
			else if (verb.equals("seed"))
			{
				seedGameRandom(Long.parseLong(command.positional.get(0)));
			}
			else if (verb.equals("fps"))
			{
				int fps = Integer.parseInt(command.positional.get(0));
				if (fps <= 0) { throw new IllegalArgumentException("fps must be positive"); }
				enableFrameStepping();
			}
			else if (verb.equals("shot"))
			{
				shot(command.positional.get(0));
			}
			else if (verb.equals("tap"))
			{
				int k = key(command.positional.get(0));
				MobilePlatform.keyPressed(k);
				dispatchQueuedInputs();
				waitFor(command, "java_frames", "ms", 1, 60);
				MobilePlatform.keyReleased(k);
				dispatchQueuedInputs();
				waitFor(command, "java_settle_frames", "settle", 2, 200);
			}
			else if (verb.equals("hold") || verb.equals("down"))
			{
				int k = key(command.positional.get(0));
				MobilePlatform.keyPressed(k);
				dispatchQueuedInputs();
				waitFor(command, "java_frames", "ms", 1, 0);
			}
			else if (verb.equals("release") || verb.equals("up"))
			{
				int k = key(command.positional.get(0));
				MobilePlatform.keyReleased(k);
				dispatchQueuedInputs();
				waitFor(command, "java_frames", "ms", 1, 0);
			}
			else { throw new IllegalArgumentException("bad script cmd: " + line); }
		}
	}

	static int key(String name)
	{
		Integer k = KEYS.get(name.toUpperCase());
		if (k == null) { throw new IllegalArgumentException("unknown key " + name); }
		return k;
	}

	static void shot(String name) throws Exception
	{
		PlatformImage img = platform.getLcdFrontbuffer();
		BufferedImage src = img.getCanvas();
		/* Copy so a concurrent MIDlet flush cannot tear the file we write. */
		BufferedImage copy = new BufferedImage(src.getWidth(), src.getHeight(), BufferedImage.TYPE_INT_RGB);
		copy.getGraphics().drawImage(src, 0, 0, null);
		File f = new File(outDir, name + ".png");
		ImageIO.write(copy, "png", f);
		long elapsed = System.currentTimeMillis() - startedAtMillis;
		shotLog.println(name + "\t" + frameCount + "\t" + elapsed);
		shotLog.flush();
		System.out.println("[drv] shot " + f.getName() + " " + src.getWidth() + "x" + src.getHeight()
			+ " frames=" + frameCount + " elapsed=" + elapsed + "ms");
	}
}
