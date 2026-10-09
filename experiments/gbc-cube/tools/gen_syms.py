#!/usr/bin/env python3
"""Generate core/src/crystal/syms.rs from pret/pokecrystal symbol files.

Usage:
    tools/gen_syms.py pokecrystal.sym pokecrystal11.sym > core/src/crystal/syms.rs

The .sym files come from the `symbols` branch of
https://github.com/pret/pokecrystal (pokecrystal.sym is Crystal 1.0 US,
pokecrystal11.sym is 1.1). Every symbol the renderer uses must have the same
address in both, or this script refuses: the cube then works on either
release without telling them apart.
"""
import re
import sys

# name -> doc line. Order is the order in the generated file.
WANTED = [
    ("wShadowOAM", "Shadow OAM: the sprites the game queued for the next frame."),
    ("wTilemap", "20x18 tile IDs of the screen as the game composed it."),
    ("wAttrmap", "20x18 CGB attributes matching wTilemap."),
    ("wOverworldMapBlocks", "The current map's blocks with a 3-block border holding connection strips (1300 bytes)."),
    ("wPlayerBGMapOffsetX", "Pixels the camera moved this step that objects haven't absorbed yet."),
    ("wPlayerBGMapOffsetY", ""),
    ("wPlayerStepFlags", ""),
    ("wBGMapAnchor", "VRAM BG map address of the screen's top-left tile."),
    ("wOverworldMapAnchor", "Address in wOverworldMapBlocks of the block at the screen's top-left (minus the half-block)."),
    ("wPlayerMetatileY", "0 or 1: which half of the anchor block the screen starts on."),
    ("wPlayerMetatileX", ""),
    ("wMapTileset", ""),
    ("wMapBorderBlock", "Drawn where wOverworldMapBlocks holds 0."),
    ("wMapHeight", "In blocks (32x32 px)."),
    ("wMapWidth", "In blocks (32x32 px)."),
    ("wTilesetBlocksBank", "ROM bank of the tileset's metatiles (16 tile IDs per block)."),
    ("wTilesetBlocksAddress", ""),
    ("wTilesetPalettes", "Pointer, in the bank of _LoadOverworldAttrmapPals, to the tileset's palette map (a nibble per tile ID)."),
    ("wBattleMode", "Nonzero in battle."),
    ("wObjectStructs", "13 object structs; the player is the first."),
    ("wMapObjects", ""),
    ("wTimeOfDayPal", ""),
    ("wVBlankOccurred", "1 while the main loop waits for VBlank in DelayFrame; the VBlank handler clears it."),
    ("wMapGroup", ""),
    ("wMapNumber", ""),
    ("wYCoord", "Player position in 16 px steps."),
    ("wXCoord", ""),
    ("wPartyCount", ""),
    ("wPartySpecies", ""),
    ("wWindowStackPointer", ""),
    ("wSpriteAnimationStructs", "10 sprite animation structs of 16 bytes (the naming screen's cursor is one)."),
    ("wSpriteAnimDataEnd", ""),
    ("wNamingScreenCurNameLength", "Naming screen: characters typed so far."),
    ("wNamingScreenMaxNameLength", ""),
    ("wNamingScreenCursorObjectPointer", "Naming screen: the cursor's sprite animation struct (Var1 column, Var2 row)."),
    ("wNamingScreenStringEntryCoord", "Naming screen: the wTilemap address the name is printed at."),
    ("wJumptableIndex", "The running screen's state (the Pokédex's DEXSTATE_*, the naming screen's...)."),
    ("wMenuDataBank", "The open menu's header bank and items table: which menu it is."),
    ("wMenuDataPointerTableAddr", ""),
    ("wMenuCursorY", "The open menu's cursor row, from 1."),
    ("wMenuItemsList", "The start menu's items: a count, then STARTMENUITEM_* values."),
    ("wPlayerName", ""),
    ("wCurPartyMon", "The party member picked (0–5)."),
    ("wPartyMon1", "Six 48-byte party structs."),
    ("wPartyMonNicknames", "Six 11-byte names, '@'-terminated."),
    ("wPokedexCaught", "A bit per species, from 1."),
    ("wPokedexSeen", ""),
    ("wPokedexOrder", "Pokédex: the species list in the current order (a union: only meaningful in the Pokédex)."),
    ("wDexListingScrollOffset", ""),
    ("wDexListingCursor", ""),
    ("hSCX", "The scroll the VBlank handler copies to rSCX for the next frame."),
    ("hSCY", ""),
    ("hWY", ""),
    ("hROMBank", ""),
    ("StartMenu.Items", "ROM: the start menu's items table (identifies the start menu)."),
    ("BaseData", "ROM: 32 bytes per species: dex number, stats, types, ..., pic size at 17."),
    ("PokemonNames", "ROM: 10 bytes per species."),
    ("PokemonPicPointers", "ROM: per species, front and back pics as (bank - PICS_FIX, address)."),
    ("UnownPicPointers", ""),
    ("FixPicBank.PicsBanks", "ROM: maps a pic pointer's bank byte to the real bank."),
    ("PokemonPalettes", "ROM: per species from 0, normal and shiny, the 2 middle colours each."),
    ("TypeNames", "ROM: pointers to type names, by type value."),
    ("Facings", "ROM: pointer table, per FACING_* value, to OAM templates (count, then y, x, attributes, tile)."),
    ("_LoadOverworldAttrmapPals", "ROM: only its bank is used (the palette maps live in that bank)."),
]


def load(path):
    syms = {}
    with open(path) as f:
        for line in f:
            line = line.split(";")[0].strip()
            if not line:
                continue
            parts = line.split()
            if len(parts) != 2 or ":" not in parts[0]:
                continue
            loc, name = parts
            bank, addr = loc.split(":")
            syms[name] = (int(bank, 16), int(addr, 16))
    return syms


def main():
    tables = [load(p) for p in sys.argv[1:]]
    if not tables:
        sys.exit(__doc__)
    out = []
    out.append("//! Pokémon Crystal RAM and ROM addresses, generated by `tools/gen_syms.py`")
    out.append("//! from the pret/pokecrystal symbol files (`symbols` branch). Don't edit by")
    out.append("//! hand. Each address is the same in Crystal 1.0 and 1.1.")
    out.append("")
    out.append("use crate::mem::Sym;")
    out.append("")
    for name, doc in WANTED:
        locs = {t.get(name) for t in tables}
        if None in locs:
            sys.exit(f"{name}: missing from a symbol file")
        if len(locs) != 1:
            sys.exit(f"{name}: differs between the symbol files: {locs}")
        bank, addr = locs.pop()
        const = name.lstrip("_")
        const = re.sub(r"(?<=[a-z0-9])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])", "_", const).upper()
        const = const.replace(".", "_")
        if doc:
            out.append(f"/// {doc}")
        out.append(f"pub const {const}: Sym = Sym::new(0x{bank:02X}, 0x{addr:04X});")
    print("\n".join(out))


if __name__ == "__main__":
    main()
