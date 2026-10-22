# Disclosure on LLM (AI) Usage in ludared

**NOTE:** This file is entirely human-written.

>"If you cannot tell where the human mind ends, and where the LLM code begins, you're not entitled to an opinion on AI usage in this project."

This is my personal opinion as an experienced developer.

However, I do agree wholeheartedly that slop is becoming a big issue, and everytime I open [r/romhacking](https://old.reddit.com/r/romhacking/), I want to claw my eyes out. A few sensible and level-headed voices told me that being clear on the boundaries between what a human built, and what an LLM wrote, would help dissuade people's fears.

As mentioned in the project's [AGENTS.md](./AGENTS.md), ludared is my passion project, it's quite literally the tool I wish I had when I started the [french Sakura Wars romhack project](https://github.com/sakurayaku/re.sakurataisen) a few years ago. Therefore, there is no incentive for me to ask "Claude, make me ludared. Make no mistakes." as it would deprive me of all the pleasure of doing it myself.

Which is not to say that I don't use "AI"[^ai-llm] either: I have a "ChatGPT chat" subscription (paid by [an employer](https://spark-club.fr)) which I can use in an unlimited manner to discuss all of my projects and ideas. In a way similar to [Rubber duck debugging](https://en.wikipedia.org/wiki/Rubber_duck_debugging), it allows me to articulate my thoughts, jot down ideas, hypotheses, and quickly get answers about technologies. For instance: "If we migrate ludared.toml from JSON to [Protobuf](https://en.wikipedia.org/wiki/Protocol_Buffers) or [Cap'n Proto](https://en.wikipedia.org/wiki/Cap%27n_Proto), what are we losing?" is something I may ask from it.

The main point is still: I know it's a tool in my toolbelt, and I know when NOT to use it. Chiefly:
- the architecture. I wanted that FS-like organization, I got it, and it works.
- the codec trait. It's simple, and it should cover all usages.
- the contracts. No matter how much an LLM could make work "Sure, pick up this leaf from another tree branch, and access it from there", I don't want it.

And there is stuff where I know it can (and probably will!) teach me:
- source streaming: right now, all source files and artifacts stay in memory. It's okay for a V1, but when I want to port ludared to the browser for public patching, I'll need streaming.
- incremental builds: seriously, I've never even done that. I know what I want out of it, but it's not every day you write your own equivalent of [GNU Make](https://en.wikipedia.org/wiki/Make_(software)).
- fuse and GUI integrations: These feel like adjacent to the main tool, and are not the kind of things I enjoy doing (nor do I have an eye for them).

But in the end, regardless of who (or what[^he-it]) writes the code, I remain the sole human responsible for reviewing, validating, and maintaining anything that makes its way into ludared.

And I take its architectural beauty[^beauty] very seriously.

# LLM Code attribution
All that said, there are indeed parts where I just pointed an LLM and said "Do this, make no mistakes". While no amount of "I still review the code the LLMs hand me, like I would a human contributor" will convince AI-adverse people, I still agree that proper attribution can dissuage fears of vibecode.

## General: Documentation & tedious work

Stuff where having an LLM under hand just beats opening the online documentation, finding precisely what I need, then coming back to my text editor. Seriously, sometimes, just asking "What's the [serde](https://github.com/serde-rs/serde) syntax again to omit an empty field?" is all you need.

Likewise, when looking for a crate to use, an LLM works better (and faster) at identifying what's available, and analyzing both the pain points I'm wary about and the features I'd prefer having (like a vendored lua implementation, instead of relying on the user's local install, which is a whole other can of worms).

There's also tedious rewrites, like "Actually that function should receive a struct, not seven arguments" which is a perfect task for an LLM, that benefits little from any human interaction that's not "Thanks for the changes, I'll review them manually".

## General: Documentation
I'm [not confident in the way I explain things](https://en.wikipedia.org/wiki/Impostor_syndrome), which I believe always strays either to "This stuff is evident, no need to explain it" or "I'm gonna tell you how bytes work", which I believe (falsely?) may become hard to read for humans. Therefore, I often use LLMs to write documentation, rustdoc, rewrite entire paragraphs, proof-check me, etc...

Therefore, this issue is not about "I don't care" but rather about "I believe the LLM can rewrite things in a straight way that even non-native english speakers will understand and follow easily". Maybe that will change that over time as I gain more confidence in my work.

## General: Tests
A lot of tests use harnesses, fixtures, setups,.. It's not something I enjoy doing, at all, nor is it a "necessary" part of ludared. It's very helpful to make sure a mutation won't break the functionalities, or that scenarios will always run properly, but I don't believe it's what makes ludared an interesting software.

## Codec: `std/cdrom/iso9660`
* Done by: `DeepSeek`

The first pass was done using the crate [hadris-rs](https://github.com/hxyulin/hadris). However, it proved to adhere too strictly to the [ISO 9660](https://en.wikipedia.org/wiki/ISO_9660) standards, and it would not preserve data from the [Sakura Taisen saturn CD](http://redump.org/disc/130400) and later reject the [PSP UMD](http://redump.org/disc/3313/).

In the first case, the issue was not preserving the [System Area](https://en.wikipedia.org/wiki/ISO_9660#Top_level). In the second, the [Volume Descriptor Version](https://en.wikipedia.org/wiki/ISO_9660#Volume_descriptor_set) was set to 2, which was incompatible with the standard. While I did know [how I wanted to solve those issues](src/codecs/cdrom/iso9660/metadata.rs), it quickly appeared there would be more data, beyond the CD's filesystem, that would require preservation in order to build functional CD images, for future platforms, and [hadris-rs](https://github.com/hxyulin/hadris) would be too limited for our usage.

I decided to throw [DeepSeek](https://en.wikipedia.org/wiki/DeepSeek) at it:
- the point of ludared is not for me to be able to navigate all the binary formats it will support. I've gained a new understanding of the [ISO 9660](https://en.wikipedia.org/wiki/ISO_9660), which is always a pleasure, but rewriting the codec from scratch to suit my needs would be akin to rewriting a project similar in scope to [hadris-rs](https://github.com/hxyulin/hadris).
- the "trick" behind how ludared would preserve non-filesystem artifacts was already in place.
- the LLM could write a harness to access both CD images, and compare them bit-by-bit to test the codec's rebuild capabilities, and identify what was wrong. Which was a nice boon, since it inspired the [codec E2E tests](./CODEC_CONTRACT.md).

The diff is around 7000 lines, with a lot of tests (and still in review as I write those lines, because that asshole tried to break trait contracts). While it's got obvious style and boundary issues (hence why I'm adding a [ludared-codec-authoring skill](skills/ludared-codec-authoring/SKILL.md) to the repository), the implementation is sound, and a local build allowed for the [first beta release of the Sakura Taisen french patch project](https://github.com/sakurayaku/re.sakurataisen/releases/tag/b20261018).

## Codec: `std/nintendo/*`
* Done by: `DeepSeek`

The second game that birthed my itch for ludared is [Theatrhythm Dragon Quest](https://en.wikipedia.org/wiki/Theatrhythm_Dragon_Quest) for the 3DS. I bought the game in Japan in 2017 and had lots of fun playing it, but still, the (very few) Japanese-only texts always led me to think something could be done. I looked through the [RomFS](https://www.3dbrew.org/wiki/RomFS), and lo and behold! A lot of strings were contained in...[CSV files](https://en.wikipedia.org/wiki/Comma-separated_values)!?  
  
Modifying them produced (of course!) no interesting results beyond garbage. Further trials made me deduce the font for that part of the game maybe wasn't carrying the characters I was adding, so I went to check the font and...couldn't find it among the HUNDREDS OF ARCHIVES themselves containing HUNDREDS OF FILES until I could find HUNDREDS OF [3DS font files](https://www.3dbrew.org/wiki/BCFNT). Yeah, I really needed ludared to organize that.

My first ludared-less dives were done using [ctrtool](https://github.com/3DSGuy/Project_CTR) and [bcfnt.py](https://github.com/ObsidianX/3dstools), which both seemed like the definitive tools for that, but now I needed that knowledge in ludared to organize my work.

I decided to throw [DeepSeek](https://en.wikipedia.org/wiki/DeepSeek) at it:
- again, the point is not for me to know all the ins and outs of [CCI](https://www.3dbrew.org/wiki/CCI), [NCCH](https://www.3dbrew.org/wiki/NCCH), [BCFNT](https://www.3dbrew.org/wiki/BCFNT),... Which isn't to say that I won't have a look in it either: the [NCCH](https://www.3dbrew.org/wiki/NCCH) codec proved very annoying because padding had to be preserved, and strategies and decisions had to be made about it. I made those decisions, the LLM followed.
- I'm still the one who made the decisions about how stuff would be preserved.
- again, the capacity of an LLM to write a harness that it'll use to test its own code is very good.
- the TRUE value of that code resides in the labor of their [original](https://github.com/devkitPro/3dstools) [authors](https://github.com/ObsidianX/3dstools), not in me translating those algorithms to rust, for which I will NEVER ever pretend any ownership nor authorship (nor even "knowledge" lol). Plus I believe that's the kind of "tedious" work that's good to give to an LLM, since it won't be asked to make architectural decisions beyond the limited scope of a codec.
- lol okay I won't even hide it, I just hate reading python.

# LLM Restrictions

As said earlier, LLMs are not given free reign over this codebase. There are even strict rules of what no LLM is allowed to do:
- No LLM is given commit or push privileges. The only exception is for WIP commits on non-master branches.
- No LLM is given architecture decision privileges: it may offer proposals, but a human must decide.
- No LLM is allowed to modify public APIs or break established contracts without explicit human approval.

***
[^ai-llm]: I hate how the term “AI” has been taken over by LLMs. Sure, that's artificial, but the "intelligence" part is a marketing stretch: it's just [Markov chains](https://en.wikipedia.org/wiki/Markov_chain) on steroids, and we've had funnier ones [for years](https://en.wikipedia.org/wiki/Horse_ebooks).
[^beauty]: You are welcome to disagree, and I'll be happy to discuss that, but you're not welcome to tell my wife is ugly and you'd have chosen another one.
[^he-it]: My native language is french, and we don't have a "neutral" gender like the english "it". Therefore, I mostly think of things in "he or she", very rarely in "it". I try my best to always use "it" in this document, but were I to use an "he" to describe an LLM, please don't conflate it as me advocating for the "humanity" of LLMs. Nope, LLMs aren't human, just like my shell scripts are not conscious, and they have no rights beyond the `+x` I may bestow them.
