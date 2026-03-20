use core::ops::Deref;
use std::ops::Index;

/// A card
///
/// This consumes just one byte, but acts like a pointer into a fixed
/// table of card data through auto-deref to `&CardData`
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct Card(u8);

impl Deref for Card {
    type Target = CardData;

    fn deref(&self) -> &Self::Target {
        &CARDS[self.0 as usize]
    }
}

/// Data about a card
pub struct CardData {
    /// Card number within suit: 1..=13
    pub num: u8,
    /// Card number as a `char`: one of "A23456789XJQK"
    pub num_ch: char,
    /// Card suit: 0: hearts, 1: diamonds, 2: clubs, 3: spades
    pub suit: u8,
    /// Card suit as a 'char': 'H', 'D', 'C', 'S'
    pub suit_ch: char,
    /// `true` if a red card, otherwise `false`
    pub is_red: bool,
    /// HFB colour to use for the card
    pub hfb: u16,
    /// Next card within suit, or `None` if this is a King
    pub inc: Option<Card>,
    /// Previous card within suit, or `None` if this is an Ace
    pub dec: Option<Card>,
    /// Same card in next suit in sequence, wrapping
    pub inc_suit: Card,
    /// Same card in previous suit in sequence, wrapping
    pub dec_suit: Card,
}

impl CardData {
    const fn genr(i: u8) -> CardData {
        let num = i % 13 + 1;
        let suit = i / 13 % 4;
        CardData {
            num,
            num_ch: CARD_NAME[num as usize - 1],
            suit,
            suit_ch: CARD_SUIT[suit as usize],
            is_red: suit < 2,
            hfb: CARD_HFB[suit as usize],
            inc: if num == 13 {
                None
            } else {
                Some(Card(suit * 13 + num))
            },
            dec: if num == 1 {
                None
            } else {
                Some(Card(suit * 13 + num - 2))
            },
            inc_suit: Card((suit + 1) % 4 + num - 1),
            dec_suit: Card((suit + 3) % 4 + num - 1),
        }
    }
}

const CARD_NAME: [char; 13] = [
    'A', '2', '3', '4', '5', '6', '7', '8', '9', 'X', 'J', 'Q', 'K',
];
const CARD_SUIT: [char; 4] = ['H', 'D', 'C', 'S'];
const CARD_HFB: [u16; 4] = [172, 173, 4, 6];

const CARDS: [CardData; 52] = [
    CardData::genr(0),
    CardData::genr(1),
    CardData::genr(2),
    CardData::genr(3),
    CardData::genr(4),
    CardData::genr(5),
    CardData::genr(6),
    CardData::genr(7),
    CardData::genr(8),
    CardData::genr(9),
    CardData::genr(10),
    CardData::genr(11),
    CardData::genr(12),
    CardData::genr(13),
    CardData::genr(14),
    CardData::genr(15),
    CardData::genr(16),
    CardData::genr(17),
    CardData::genr(18),
    CardData::genr(19),
    CardData::genr(20),
    CardData::genr(21),
    CardData::genr(22),
    CardData::genr(23),
    CardData::genr(24),
    CardData::genr(25),
    CardData::genr(26),
    CardData::genr(27),
    CardData::genr(28),
    CardData::genr(29),
    CardData::genr(30),
    CardData::genr(31),
    CardData::genr(32),
    CardData::genr(33),
    CardData::genr(34),
    CardData::genr(35),
    CardData::genr(36),
    CardData::genr(37),
    CardData::genr(38),
    CardData::genr(39),
    CardData::genr(40),
    CardData::genr(41),
    CardData::genr(42),
    CardData::genr(43),
    CardData::genr(44),
    CardData::genr(45),
    CardData::genr(46),
    CardData::genr(47),
    CardData::genr(48),
    CardData::genr(49),
    CardData::genr(50),
    CardData::genr(51),
];

/// Trait that must be implemented to provide random numbers to the
/// relevant methods
pub trait Rand {
    fn get(&mut self, _: std::ops::Range<u32>) -> u32;
}

/// A hand of cards, in order
#[derive(Default, Clone, Eq, PartialEq)]
pub struct Hand {
    vec: Vec<Card>,
}

impl Hand {
    /// Number of cards in the hand
    pub fn len(&self) -> usize {
        self.vec.len()
    }

    /// Is the hand empty?
    pub fn is_empty(&self) -> bool {
        self.vec.is_empty()
    }

    /// Add a whole deck of cards in order to the end of this hand
    pub fn add_deck(&mut self) {
        for i in 0..52 {
            self.vec.push(Card(i));
        }
    }

    /// Add a card to the end of the hand
    pub fn add(&mut self, card: Card) {
        self.vec.push(card);
    }

    /// Pick a random card from the hand, removing it from the hand,
    /// or return `None` if the hand is empty
    pub fn pick_random(&mut self, rand: &mut impl Rand) -> Option<Card> {
        if self.is_empty() {
            None
        } else {
            let i = rand.get(0..self.len() as u32) as usize;
            Some(self.vec.remove(i))
        }
    }

    /// Pick first card out of the hand, removing it and return it, or
    /// return `None` if the hand is empty.
    pub fn pick_first(&mut self) -> Option<Card> {
        if self.is_empty() {
            None
        } else {
            Some(self.vec.remove(0))
        }
    }

    /// Pick last card out of the hand, removing it and return it, or
    /// return `None` if the hand is empty.
    pub fn pick_last(&mut self) -> Option<Card> {
        self.vec.pop()
    }

    /// Pick the head `cnt` cards, removing them, and return them as a
    /// new `Hand`, in the same order.  If there are not enough cards
    /// then as many cards are returned as possible.
    pub fn pick_head(&mut self, cnt: usize) -> Hand {
        let tail = self.vec.split_off(cnt.min(self.len()));
        let head = std::mem::replace(&mut self.vec, tail);
        Hand { vec: head }
    }

    /// Pick the tail `cnt` cards, removing them, and return them as a
    /// new `Hand`, in the same order.  If there are not enough cards
    /// then as many cards are returned as possible.
    pub fn pick_tail(&mut self, cnt: usize) -> Hand {
        let tail = self.vec.split_off(cnt.min(self.len()));
        Hand { vec: tail }
    }

    /// Return the first card in the hand
    pub fn peek_first(&self) -> Option<Card> {
        self.vec.first().copied()
    }

    /// Return the last card in the hand
    pub fn peek_last(&self) -> Option<Card> {
        self.vec.last().copied()
    }

    /// Get the first `cnt` cards out of the hand without removing
    /// them, and return them as a new `Hand`, in the same order.  If
    /// there are not enough cards then as many cards are returned as
    /// possible.
    pub fn peek_head(&mut self, cnt: usize) -> Hand {
        Hand {
            vec: self.vec[..cnt.min(self.len())].to_vec(),
        }
    }

    /// Get the last `cnt` cards out of the hand without removing
    /// them, and return them as a new `Hand`, in the same order.  If
    /// there are not enough cards then as many cards are returned as
    /// possible.
    pub fn peek_tail(&mut self, cnt: usize) -> Hand {
        let len = self.len();
        let cnt = cnt.min(len);
        Hand {
            vec: self.vec[len - cnt..].to_vec(),
        }
    }

    /// Shuffle the cards in the hand
    pub fn shuffle(&mut self, rand: &mut impl Rand) {
        let mut tmp = Vec::with_capacity(self.len());
        while let Some(card) = self.pick_random(rand) {
            tmp.push(card);
        }
        self.vec = tmp;
    }

    pub fn pack(&self, out: &mut Vec<u8>) {
        out.push(self.vec.len() as u8);
        for c in &self.vec {
            out.push(c.0);
        }
    }

    pub fn unpack(p: &mut &[u8]) -> Self {
        let len = p[0] as usize;
        let mut vec = Vec::with_capacity(len);
        for i in 0..len {
            vec.push(Card(p[1 + i]));
        }
        *p = &p[1 + len..];
        Self { vec }
    }
}

impl Index<usize> for Hand {
    type Output = Card;

    fn index(&self, index: usize) -> &Self::Output {
        &self.vec[index]
    }
}
