//! Curated starting list of literary awards. The full/most-recent winner list
//! is refreshed from Wikipedia once a month into a local JSON cache.

/// One award with a few well-known recent winners.
#[derive(Debug, Clone)]
pub struct Award {
    pub name: &'static str,
    pub language: &'static str,
    pub region: &'static str,
    pub wikipedia_page: &'static str,
    pub winners: &'static [(&'static str, &'static str, &'static str)],
}

pub fn curated_awards() -> Vec<Award> {
    vec![
        Award {
            name: "Booker Prize",
            language: "English",
            region: "UK / Ireland",
            wikipedia_page: "Booker_Prize",
            winners: &[
                ("2024", "Orbital", "Samantha Harvey"),
                ("2023", "Prophet Song", "Paul Lynch"),
                (
                    "2022",
                    "The Seven Moons of Maali Almeida",
                    "Shehan Karunatilaka",
                ),
                ("2021", "The Promise", "Damon Galgut"),
            ],
        },
        Award {
            name: "International Booker Prize",
            language: "English (translated)",
            region: "Global",
            wikipedia_page: "International_Booker_Prize",
            winners: &[
                ("2024", "Kairos", "Jenny Erpenbeck"),
                ("2023", "Time Shelter", "Georgi Gospodinov"),
                ("2022", "Tomb of Sand", "Geetanjali Shree"),
            ],
        },
        Award {
            name: "Pulitzer Prize for Fiction",
            language: "English",
            region: "USA",
            wikipedia_page: "Pulitzer_Prize_for_Fiction",
            winners: &[
                ("2024", "Night Watch", "Jayne Anne Phillips"),
                ("2023", "Trust", "Hernan Diaz"),
                ("2022", "The Netanyahus", "Joshua Cohen"),
                ("2021", "The Night Watchman", "Louise Erdrich"),
            ],
        },
        Award {
            name: "Jnanpith Award",
            language: "Malayalam & Indian languages",
            region: "India",
            wikipedia_page: "Jnanpith_Award",
            winners: &[
                ("2019", "O. N. V. Kurup", "Malayalam poetry"),
                ("2018", "Amitav Ghosh", "English fiction"),
                ("2016", "M. T. Vasudevan Nair", "Malayalam fiction"),
            ],
        },
        Award {
            name: "Kerala Sahitya Akademi Award",
            language: "Malayalam",
            region: "Kerala, India",
            wikipedia_page: "Kerala_Sahitya_Akademi_Award",
            winners: &[
                (
                    "2022",
                    "Malayalam fiction & poetry winners",
                    "Kerala Sahitya Akademi",
                ),
                (
                    "2021",
                    "Malayalam fiction & poetry winners",
                    "Kerala Sahitya Akademi",
                ),
            ],
        },
        Award {
            name: "Sahitya Akademi Award",
            language: "English & Malayalam",
            region: "India",
            wikipedia_page: "Sahitya_Akademi_Award",
            winners: &[
                ("2023", "English & Malayalam winners", "Sahitya Akademi"),
                ("2022", "English & Malayalam winners", "Sahitya Akademi"),
            ],
        },
        Award {
            name: "Sahitya Akademi Yuva Puraskar",
            language: "English & Malayalam",
            region: "India",
            wikipedia_page: "Sahitya_Akademi_Yuva_Puraskar",
            winners: &[("2023", "Young writers award", "Sahitya Akademi")],
        },
        Award {
            name: "National Book Award for Fiction",
            language: "English",
            region: "USA",
            wikipedia_page: "National_Book_Award_for_Fiction",
            winners: &[
                ("2023", "The Heaven & Earth Grocery Store", "James McBride"),
                ("2022", "The Rabbit Hutch", "Tess Gunty"),
            ],
        },
        Award {
            name: "Women's Prize for Fiction",
            language: "English",
            region: "UK",
            wikipedia_page: "Women's_Prize_for_Fiction",
            winners: &[
                ("2024", "Orbital", "Samantha Harvey"),
                ("2023", "Trust", "Hernan Diaz"),
            ],
        },
        Award {
            name: "JCB Prize for Literature",
            language: "English / Indian translations",
            region: "India",
            wikipedia_page: "JCB_Prize_for_Literature",
            winners: &[("2023", "The Other Woman", "K. R. Meera")],
        },
    ]
}
