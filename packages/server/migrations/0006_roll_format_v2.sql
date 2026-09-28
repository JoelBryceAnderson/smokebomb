-- Roll format v2 (docs/SIM_SPEC.md, decision H6): up to 10 dice, and the
-- Pass the Pot die, which is signed as raw d6 values. The die is stored by
-- its wire name because Pass the Pot and d6 share a value range.
ALTER TABLE rolls ADD COLUMN die TEXT;
UPDATE rolls SET die = 'd' || die_sides;
ALTER TABLE rolls
    ALTER COLUMN die SET NOT NULL,
    ADD CONSTRAINT rolls_die_check
        CHECK (die IN ('d4', 'd6', 'd8', 'd10', 'd12', 'd20', 'd100', 'pass_the_pot')),
    DROP COLUMN die_sides,
    DROP CONSTRAINT rolls_dice_check,
    ADD CONSTRAINT rolls_dice_check CHECK (cardinality(dice) BETWEEN 1 AND 10);

-- The signed encoding's version byte. Rows written before this migration
-- were v1; new rows must say which format their signature covers.
ALTER TABLE rolls ADD COLUMN format_version SMALLINT NOT NULL DEFAULT 1;
ALTER TABLE rolls ALTER COLUMN format_version DROP DEFAULT;
