//! Insertion choices from the desktop X岛 composer, in website order.
//! Sources (retrieved 2026-10-23):
//! https://www.nmbxd1.com/Public/Js/h.desktop.js — initPostForm / emotList (87).
//! https://www.nmbxd1.com/t/69665764 — inline “颜文字补丁” (12 + 5 named choices).
//! The empty “无” option is omitted. Preserve all whitespace and Unicode variants.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kaomoji {
    pub label: &'static str,
    pub value: &'static str,
}

pub const KAOMOJI: &[Kaomoji] = &[
    Kaomoji {
        label: "|∀ﾟ",
        value: "|∀ﾟ",
    },
    Kaomoji {
        label: "(´ﾟДﾟ`)",
        value: "(´ﾟДﾟ`)",
    },
    Kaomoji {
        label: "(;´Д`)",
        value: "(;´Д`)",
    },
    Kaomoji {
        label: "(｀･ω･)",
        value: "(｀･ω･)",
    },
    Kaomoji {
        label: "(=ﾟωﾟ)=",
        value: "(=ﾟωﾟ)=",
    },
    Kaomoji {
        label: "| ω・´)",
        value: "| ω・´)",
    },
    Kaomoji {
        label: "|-` )",
        value: "|-` )",
    },
    Kaomoji {
        label: "|д` )",
        value: "|д` )",
    },
    Kaomoji {
        label: "|ー` )",
        value: "|ー` )",
    },
    Kaomoji {
        label: "|∀` )",
        value: "|∀` )",
    },
    Kaomoji {
        label: "(つд⊂)",
        value: "(つд⊂)",
    },
    Kaomoji {
        label: "(ﾟДﾟ≡ﾟДﾟ)",
        value: "(ﾟДﾟ≡ﾟДﾟ)",
    },
    Kaomoji {
        label: "(＾o＾)ﾉ",
        value: "(＾o＾)ﾉ",
    },
    Kaomoji {
        label: "(|||ﾟДﾟ)",
        value: "(|||ﾟДﾟ)",
    },
    Kaomoji {
        label: "( ﾟ∀ﾟ)",
        value: "( ﾟ∀ﾟ)",
    },
    Kaomoji {
        label: "( ´∀`)",
        value: "( ´∀`)",
    },
    Kaomoji {
        label: "(*´∀`)",
        value: "(*´∀`)",
    },
    Kaomoji {
        label: "(*ﾟ∇ﾟ)",
        value: "(*ﾟ∇ﾟ)",
    },
    Kaomoji {
        label: "(*ﾟーﾟ)",
        value: "(*ﾟーﾟ)",
    },
    Kaomoji {
        label: "(　ﾟ 3ﾟ)",
        value: "(　ﾟ 3ﾟ)",
    },
    Kaomoji {
        label: "( ´ー`)",
        value: "( ´ー`)",
    },
    Kaomoji {
        label: "( ・_ゝ・)",
        value: "( ・_ゝ・)",
    },
    Kaomoji {
        label: "( ´_ゝ`)",
        value: "( ´_ゝ`)",
    },
    Kaomoji {
        label: "(*´д`)",
        value: "(*´д`)",
    },
    Kaomoji {
        label: "(・ー・)",
        value: "(・ー・)",
    },
    Kaomoji {
        label: "(・∀・)",
        value: "(・∀・)",
    },
    Kaomoji {
        label: "(ゝ∀･)",
        value: "(ゝ∀･)",
    },
    Kaomoji {
        label: "(〃∀〃)",
        value: "(〃∀〃)",
    },
    Kaomoji {
        label: "(*ﾟ∀ﾟ*)",
        value: "(*ﾟ∀ﾟ*)",
    },
    Kaomoji {
        label: "( ﾟ∀。)",
        value: "( ﾟ∀。)",
    },
    Kaomoji {
        label: "( `д´)",
        value: "( `д´)",
    },
    Kaomoji {
        label: "(`ε´ )",
        value: "(`ε´ )",
    },
    Kaomoji {
        label: "(`ヮ´ )",
        value: "(`ヮ´ )",
    },
    Kaomoji {
        label: "σ`∀´)",
        value: "σ`∀´)",
    },
    Kaomoji {
        label: " ﾟ∀ﾟ)σ",
        value: " ﾟ∀ﾟ)σ",
    },
    Kaomoji {
        label: "ﾟ ∀ﾟ)ノ",
        value: "ﾟ ∀ﾟ)ノ",
    },
    Kaomoji {
        label: "(╬ﾟдﾟ)",
        value: "(╬ﾟдﾟ)",
    },
    Kaomoji {
        label: "(|||ﾟдﾟ)",
        value: "(|||ﾟдﾟ)",
    },
    Kaomoji {
        label: "( ﾟдﾟ)",
        value: "( ﾟдﾟ)",
    },
    Kaomoji {
        label: "Σ( ﾟдﾟ)",
        value: "Σ( ﾟдﾟ)",
    },
    Kaomoji {
        label: "( ;ﾟдﾟ)",
        value: "( ;ﾟдﾟ)",
    },
    Kaomoji {
        label: "( ;´д`)",
        value: "( ;´д`)",
    },
    Kaomoji {
        label: "(　д ) ﾟ ﾟ",
        value: "(　д ) ﾟ ﾟ",
    },
    Kaomoji {
        label: "( ☉д⊙)",
        value: "( ☉д⊙)",
    },
    Kaomoji {
        label: "(((　ﾟдﾟ)))",
        value: "(((　ﾟдﾟ)))",
    },
    Kaomoji {
        label: "( ` ・´)",
        value: "( ` ・´)",
    },
    Kaomoji {
        label: "( ´д`)",
        value: "( ´д`)",
    },
    Kaomoji {
        label: "( -д-)",
        value: "( -д-)",
    },
    Kaomoji {
        label: "(>д<)",
        value: "(>д<)",
    },
    Kaomoji {
        label: "･ﾟ( ﾉд`ﾟ)",
        value: "･ﾟ( ﾉд`ﾟ)",
    },
    Kaomoji {
        label: "( TдT)",
        value: "( TдT)",
    },
    Kaomoji {
        label: "(￣∇￣)",
        value: "(￣∇￣)",
    },
    Kaomoji {
        label: "(￣3￣)",
        value: "(￣3￣)",
    },
    Kaomoji {
        label: "(￣ｰ￣)",
        value: "(￣ｰ￣)",
    },
    Kaomoji {
        label: "(￣ . ￣)",
        value: "(￣ . ￣)",
    },
    Kaomoji {
        label: "(￣皿￣)",
        value: "(￣皿￣)",
    },
    Kaomoji {
        label: "(￣艸￣)",
        value: "(￣艸￣)",
    },
    Kaomoji {
        label: "(￣︿￣)",
        value: "(￣︿￣)",
    },
    Kaomoji {
        label: "(￣︶￣)",
        value: "(￣︶￣)",
    },
    Kaomoji {
        label: "ヾ(´ωﾟ｀)",
        value: "ヾ(´ωﾟ｀)",
    },
    Kaomoji {
        label: "(*´ω`*)",
        value: "(*´ω`*)",
    },
    Kaomoji {
        label: "(・ω・)",
        value: "(・ω・)",
    },
    Kaomoji {
        label: "( ´・ω)",
        value: "( ´・ω)",
    },
    Kaomoji {
        label: "(｀・ω)",
        value: "(｀・ω)",
    },
    Kaomoji {
        label: "(´・ω・`)",
        value: "(´・ω・`)",
    },
    Kaomoji {
        label: "(`・ω・´)",
        value: "(`・ω・´)",
    },
    Kaomoji {
        label: "( `_っ´)",
        value: "( `_っ´)",
    },
    Kaomoji {
        label: "( `ー´)",
        value: "( `ー´)",
    },
    Kaomoji {
        label: "( ´_っ`)",
        value: "( ´_っ`)",
    },
    Kaomoji {
        label: "( ´ρ`)",
        value: "( ´ρ`)",
    },
    Kaomoji {
        label: "( ﾟωﾟ)",
        value: "( ﾟωﾟ)",
    },
    Kaomoji {
        label: "(oﾟωﾟo)",
        value: "(oﾟωﾟo)",
    },
    Kaomoji {
        label: "(　^ω^)",
        value: "(　^ω^)",
    },
    Kaomoji {
        label: "(｡◕∀◕｡)",
        value: "(｡◕∀◕｡)",
    },
    Kaomoji {
        label: "/( ◕‿‿◕ )\\",
        value: "/( ◕‿‿◕ )\\",
    },
    Kaomoji {
        label: "ヾ(´ε`ヾ)",
        value: "ヾ(´ε`ヾ)",
    },
    Kaomoji {
        label: "(ノﾟ∀ﾟ)ノ",
        value: "(ノﾟ∀ﾟ)ノ",
    },
    Kaomoji {
        label: "(σﾟдﾟ)σ",
        value: "(σﾟдﾟ)σ",
    },
    Kaomoji {
        label: "(σﾟ∀ﾟ)σ",
        value: "(σﾟ∀ﾟ)σ",
    },
    Kaomoji {
        label: "|дﾟ )",
        value: "|дﾟ )",
    },
    Kaomoji {
        label: "┃電柱┃",
        value: "┃電柱┃",
    },
    Kaomoji {
        label: "ﾟ(つд`ﾟ)",
        value: "ﾟ(つд`ﾟ)",
    },
    Kaomoji {
        label: "ﾟÅﾟ )　",
        value: "ﾟÅﾟ )　",
    },
    Kaomoji {
        label: "⊂彡☆))д`)",
        value: "⊂彡☆))д`)",
    },
    Kaomoji {
        label: "⊂彡☆))д´)",
        value: "⊂彡☆))д´)",
    },
    Kaomoji {
        label: "⊂彡☆))∀`)",
        value: "⊂彡☆))∀`)",
    },
    Kaomoji {
        label: "(´∀((☆ミつ",
        value: "(´∀((☆ミつ",
    },
    // Inline page additions follow the desktop script choices.
    Kaomoji {
        label: "･ﾟ( ﾉヮ´ )",
        value: "･ﾟ( ﾉヮ´ )",
    },
    Kaomoji {
        label: "(ﾉ)`ω´(ヾ)",
        value: "(ﾉ)`ω´(ヾ)",
    },
    Kaomoji {
        label: "ᕕ( ᐛ )ᕗ",
        value: "ᕕ( ᐛ )ᕗ",
    },
    Kaomoji {
        label: "(　ˇωˇ)",
        value: "(　ˇωˇ)",
    },
    Kaomoji {
        label: "( ｣ﾟДﾟ)｣＜",
        value: "( ｣ﾟДﾟ)｣＜",
    },
    Kaomoji {
        label: "( ›´ω`‹ )",
        value: "( ›´ω`‹ )",
    },
    Kaomoji {
        label: "(;´ヮ`)7",
        value: "(;´ヮ`)7",
    },
    Kaomoji {
        label: "(`ゥ´ )",
        value: "(`ゥ´ )",
    },
    Kaomoji {
        label: "(`ᝫ´ )",
        value: "(`ᝫ´ )",
    },
    Kaomoji {
        label: "( ᑭ`д´)ᓀ))д´)ᑫ",
        value: "( ᑭ`д´)ᓀ))д´)ᑫ",
    },
    Kaomoji {
        label: "σ( ᑒ )",
        value: "σ( ᑒ )",
    },
    Kaomoji {
        label: "⁽ ˆ꒳ˆ⁾",
        value: "⁽ ˆ꒳ˆ⁾",
    },
    // Named options use different display labels and insertion text.
    Kaomoji {
        label: "齐齐蛤尔",
        value: "(`ヮ´ )σ`∀´) ﾟ∀ﾟ)σ",
    },
    Kaomoji {
        label: "大嘘",
        value: "吁~~~~　　rnm，退钱！\n\u{a0}　　　/　　　/ \n(　ﾟ 3ﾟ) `ー´) `д´) `д´)",
    },
    Kaomoji {
        label: "防剧透",
        value: "[h] [/h]",
    },
    Kaomoji {
        label: "骰子",
        value: "[n]",
    },
    Kaomoji {
        label: "高级骰子",
        value: "[n,m]",
    },
];

#[cfg(test)]
mod tests {
    use super::KAOMOJI;

    #[test]
    fn website_snapshot_preserves_order_and_exact_text() {
        assert_eq!(KAOMOJI.len(), 104);
        // FNV-1a over UTF-8 label, NUL, value, 0xff for each source entry.
        // This covers every entry, including ordering and significant whitespace.
        let expected = 0x7e601a8edf7d8ffe_u64;
        let mut actual = 0xcbf29ce484222325_u64;
        for entry in KAOMOJI {
            for byte in entry
                .label
                .bytes()
                .chain([0])
                .chain(entry.value.bytes())
                .chain([0xff])
            {
                actual = (actual ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn website_patch_and_named_insertions_are_included() {
        assert_eq!(KAOMOJI[86].value, "(´∀((☆ミつ");
        assert_eq!(KAOMOJI[87].value, "･ﾟ( ﾉヮ´ )");
        assert_eq!(KAOMOJI[98].value, "⁽ ˆ꒳ˆ⁾");
        assert_eq!(KAOMOJI[99].label, "齐齐蛤尔");
        assert_eq!(KAOMOJI[99].value, "(`ヮ´ )σ`∀´) ﾟ∀ﾟ)σ");
        assert_eq!(KAOMOJI[100].label, "大嘘");
        assert_eq!(
            KAOMOJI[100].value,
            "吁~~~~　　rnm，退钱！\n\u{a0}　　　/　　　/ \n(　ﾟ 3ﾟ) `ー´) `д´) `д´)"
        );
        assert_eq!(KAOMOJI[101].value, "[h] [/h]");
        assert_eq!(KAOMOJI[102].value, "[n]");
        assert_eq!(KAOMOJI[103].value, "[n,m]");
    }

    #[test]
    fn significant_spaces_and_backslash_are_preserved() {
        assert_eq!(KAOMOJI[19].value, "(\u{3000}ﾟ 3ﾟ)");
        assert_eq!(KAOMOJI[34].value, " ﾟ∀ﾟ)σ");
        assert_eq!(KAOMOJI[74].value, "/( ◕‿‿◕ )\\");
        assert_eq!(KAOMOJI[82].value, "ﾟÅﾟ )\u{3000}");
        assert!(
            KAOMOJI[100]
                .value
                .contains("\n\u{a0}\u{3000}\u{3000}\u{3000}/")
        );
    }
}
